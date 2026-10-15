//! In-app updates on top of the official Tauri updater plugin.
//!
//! The plugin fetches `latest.json` from GitHub Releases over TLS, picks the
//! `{os}-{arch}-{installer}` entry and verifies the minisign signature of the
//! downloaded package against the public key embedded in the application
//! (`requireSignedVersion` additionally binds the signature to the version).
//!
//! This module adds Elyvon's policy (see `elyvon-update`):
//! * nothing happens without the user: check → show notes → user downloads →
//!   user installs → user restarts;
//! * SemVer decisions without downgrades or stable → pre-release moves;
//! * the downloaded artifact must be exactly the one built for this
//!   platform, architecture and installer kind;
//! * installation and restart are refused while the project has unsaved
//!   changes (on Windows the installer closes the application).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use elyvon_update::manifest::check_artifact_url;
use elyvon_update::{Installer, Platform, STUDIO_NAMING, UpdateDecision, decide};
use semver::Version;
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::oneshot;
use ts_rs::TS;

use crate::error::{ErrorCode, IpcError, IpcResult};

/// Value of `plugins.updater.pubkey` in `tauri.conf.json` until the
/// maintainer generates the release signing key (see `docs/RELEASES.md`).
pub const UNCONFIGURED_PUBKEY: &str = "UNCONFIGURED";

/// Stable installations never auto-offer pre-releases in Phase 1.
const ALLOW_PRERELEASE: bool = false;

/// Static facts about this installation that decide whether updating is possible.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdaterEnvironment {
    pub current_version: String,
    pub platform: Option<Platform>,
    pub platform_label: Option<String>,
    pub installer: Option<Installer>,
    pub pubkey_configured: bool,
    pub debug_build: bool,
    /// `None` when updates can be checked; otherwise why not.
    pub unavailable_reason: Option<String>,
}

impl UpdaterEnvironment {
    pub fn new(
        current_version: &str,
        platform: Option<Platform>,
        installer: Option<Installer>,
        pubkey: &str,
        debug_build: bool,
    ) -> Self {
        let pubkey_configured = !pubkey.trim().is_empty() && pubkey.trim() != UNCONFIGURED_PUBKEY;
        let unavailable_reason = if debug_build {
            Some("Updates are disabled in development builds.".to_owned())
        } else if !pubkey_configured {
            Some(
                "This build was produced without an update signing key, so it cannot verify updates. \
                 Download new versions manually from GitHub Releases."
                    .to_owned(),
            )
        } else if platform.is_none() {
            Some(
                "This operating system or CPU architecture is not supported by the updater."
                    .to_owned(),
            )
        } else if installer.is_none() {
            Some(
                "This copy was not installed from an Elyvon installer, so it cannot update itself."
                    .to_owned(),
            )
        } else {
            None
        };
        Self {
            current_version: current_version.to_owned(),
            platform,
            platform_label: platform.map(|p| p.release_label().to_owned()),
            installer,
            pubkey_configured,
            debug_build,
            unavailable_reason,
        }
    }

    pub fn detect<R: Runtime>(app: &AppHandle<R>) -> Self {
        let pubkey = app
            .config()
            .plugins
            .0
            .get("updater")
            .and_then(|u| u.get("pubkey"))
            .and_then(|k| k.as_str())
            .unwrap_or_default()
            .to_owned();
        Self::new(
            &app.package_info().version.to_string(),
            Platform::current(),
            current_installer(),
            &pubkey,
            cfg!(debug_assertions),
        )
    }
}

/// The installer kind this copy was installed with, as stamped into the
/// binary by the Tauri bundler.
fn current_installer() -> Option<Installer> {
    use tauri::utils::config::BundleType;
    match tauri::utils::platform::bundle_type()? {
        BundleType::Nsis => Some(Installer::Nsis),
        BundleType::Msi => Some(Installer::Msi),
        BundleType::App | BundleType::Dmg => Some(Installer::App),
        BundleType::Deb => Some(Installer::Deb),
        BundleType::AppImage => Some(Installer::AppImage),
        BundleType::Rpm => None,
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub platform_label: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum UpdateCheckResult {
    Unavailable {
        reason: String,
    },
    UpToDate {
        current_version: String,
    },
    /// A newer pre-release exists but stable installations do not move to it.
    PrereleaseSkipped {
        current_version: String,
        version: String,
    },
    Available {
        info: UpdateInfo,
    },
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum DownloadEvent {
    Started {
        #[ts(type = "number | null")]
        content_length: Option<u64>,
    },
    Progress {
        #[ts(type = "number")]
        downloaded: u64,
        #[ts(type = "number | null")]
        content_length: Option<u64>,
    },
    /// Downloaded and signature verified; ready to install.
    Verified,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstallResult {
    /// macOS/Linux: the new version runs after a restart. (On Windows the
    /// installer takes over and this result is never received.)
    pub restart_required: bool,
}

#[derive(Default)]
#[allow(missing_debug_implementations)] // `Update` does not implement Debug
pub struct UpdaterState {
    pending: Mutex<Option<Update>>,
    downloaded: Mutex<Option<(String, Vec<u8>)>>,
    cancel: Mutex<Option<oneshot::Sender<()>>>,
    busy: tokio::sync::Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn map_plugin_error(e: tauri_plugin_updater::Error) -> IpcError {
    use tauri_plugin_updater::Error as E;
    let (code, message) = match &e {
        E::ReleaseNotFound => (
            ErrorCode::NoRelease,
            "No published release was found on the update server.".to_owned(),
        ),
        E::TargetNotFound(_) | E::TargetsNotFound(_) => (
            ErrorCode::NoRelease,
            "The latest release does not include an update for this platform and installer."
                .to_owned(),
        ),
        E::Reqwest(_) | E::Network(_) => (
            ErrorCode::Network,
            format!(
                "Could not reach the update server. Check your connection and try again. ({e})"
            ),
        ),
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion => (
            ErrorCode::UpdateRejected,
            format!("The update failed signature verification and was rejected. ({e})"),
        ),
        E::Serialization(_) | E::Semver(_) | E::InvalidUpdaterFormat => (
            ErrorCode::UpdateRejected,
            format!("The update metadata is invalid and was rejected. ({e})"),
        ),
        _ => (ErrorCode::Internal, e.to_string()),
    };
    IpcError::new(code, message)
}

/// Checks the update endpoint once. Never downloads anything.
pub async fn check<R: Runtime>(
    app: &AppHandle<R>,
    state: &UpdaterState,
) -> IpcResult<UpdateCheckResult> {
    let env = UpdaterEnvironment::detect(app);
    if let Some(reason) = env.unavailable_reason {
        return Ok(UpdateCheckResult::Unavailable { reason });
    }
    let (Some(platform), Some(installer)) = (env.platform, env.installer) else {
        return Ok(UpdateCheckResult::Unavailable {
            reason: "unsupported installation".into(),
        });
    };
    let _busy = state
        .busy
        .try_lock()
        .map_err(|_| IpcError::new(ErrorCode::Busy, "An update operation is already running."))?;

    // Remember what the policy decided so a skipped pre-release can be reported.
    let decision: Arc<Mutex<Option<(UpdateDecision, String)>>> = Arc::default();
    let seen = Arc::clone(&decision);
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .version_comparator(move |current, remote| {
            let d = decide(&current, &remote.version, ALLOW_PRERELEASE);
            *lock(&seen) = Some((d, remote.version.to_string()));
            d == UpdateDecision::Install
        })
        .build()
        .map_err(map_plugin_error)?;

    log::info!(
        "checking for updates ({}, {:?})",
        platform.release_label(),
        installer
    );
    let update = updater.check().await.map_err(map_plugin_error)?;

    let Some(update) = update else {
        let current = env.current_version;
        return Ok(match lock(&decision).take() {
            Some((UpdateDecision::PrereleaseNotAllowed, version)) => {
                UpdateCheckResult::PrereleaseSkipped {
                    current_version: current,
                    version,
                }
            }
            _ => UpdateCheckResult::UpToDate {
                current_version: current,
            },
        });
    };

    // Defence in depth on top of the plugin's own target selection.
    let current =
        Version::parse(&env.current_version).map_err(|e| IpcError::internal(e.to_string()))?;
    let candidate = Version::parse(&update.version).map_err(|_| {
        IpcError::new(
            ErrorCode::UpdateRejected,
            "The update version is not valid SemVer.",
        )
    })?;
    if decide(&current, &candidate, ALLOW_PRERELEASE) != UpdateDecision::Install {
        return Err(IpcError::new(
            ErrorCode::UpdateRejected,
            "The offered version is not an upgrade.",
        ));
    }
    let key = platform.updater_key(installer);
    check_artifact_url(
        &STUDIO_NAMING,
        &update.version,
        platform,
        installer,
        &key,
        update.download_url.as_str(),
    )
    .map_err(|e| {
        IpcError::new(
            ErrorCode::UpdateRejected,
            format!("The update was rejected: {e}"),
        )
    })?;

    let info = UpdateInfo {
        current_version: env.current_version,
        version: update.version.clone(),
        notes: update.body.clone(),
        date: update.date.map(|d| d.to_string()),
        platform_label: platform.release_label().to_owned(),
    };
    log::info!("update {} available", info.version);
    *lock(&state.downloaded) = None;
    *lock(&state.pending) = Some(update);
    Ok(UpdateCheckResult::Available { info })
}

/// Downloads and verifies the pending update. Cancellable with [`cancel`].
pub async fn download(state: &UpdaterState, events: Channel<DownloadEvent>) -> IpcResult<()> {
    let _busy = state
        .busy
        .try_lock()
        .map_err(|_| IpcError::new(ErrorCode::Busy, "An update operation is already running."))?;
    let update = lock(&state.pending)
        .clone()
        .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "Check for updates first."))?;

    let (tx, rx) = oneshot::channel();
    *lock(&state.cancel) = Some(tx);

    let mut downloaded: u64 = 0;
    let mut started = false;
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let progress_events = events.clone();
    let transfer = update.download(
        move |chunk, total| {
            if !started {
                started = true;
                let _ = progress_events.send(DownloadEvent::Started {
                    content_length: total,
                });
            }
            downloaded += chunk as u64;
            // Throttle IPC traffic to ~10 messages per second.
            if last_emit.elapsed() >= Duration::from_millis(100) || Some(downloaded) == total {
                last_emit = Instant::now();
                let _ = progress_events.send(DownloadEvent::Progress {
                    downloaded,
                    content_length: total,
                });
            }
        },
        || {},
    );

    let result = tokio::select! {
        r = transfer => r.map_err(map_plugin_error),
        _ = rx => Err(IpcError::new(ErrorCode::Cancelled, "The download was cancelled.")),
    };
    *lock(&state.cancel) = None;
    let bytes = result?;
    log::info!(
        "update {} downloaded and verified ({} bytes)",
        update.version,
        bytes.len()
    );
    *lock(&state.downloaded) = Some((update.version.clone(), bytes));
    let _ = events.send(DownloadEvent::Verified);
    Ok(())
}

pub fn cancel(state: &UpdaterState) -> bool {
    lock(&state.cancel)
        .take()
        .is_some_and(|tx| tx.send(()).is_ok())
}

/// Installs the verified package. The caller guarantees there are no unsaved changes.
pub fn install(state: &UpdaterState) -> IpcResult<InstallResult> {
    let update = lock(&state.pending)
        .clone()
        .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "Check for updates first."))?;
    let (version, bytes) = lock(&state.downloaded)
        .take()
        .ok_or_else(|| IpcError::new(ErrorCode::NotFound, "Download the update first."))?;
    if version != update.version {
        return Err(IpcError::new(
            ErrorCode::UpdateRejected,
            "The downloaded package does not match the update.",
        ));
    }
    log::info!("installing update {version}");
    // `install` re-verifies the signature before touching the installation.
    update.install(&bytes).map_err(map_plugin_error)?;
    *lock(&state.pending) = None;
    Ok(InstallResult {
        restart_required: !cfg!(target_os = "windows"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use elyvon_update::{Arch, Os};

    const LINUX: Platform = Platform {
        os: Os::Linux,
        arch: Arch::X86_64,
    };

    #[test]
    fn release_install_with_key_is_updatable() {
        let env = UpdaterEnvironment::new(
            "0.1.0",
            Some(LINUX),
            Some(Installer::AppImage),
            "dW50cnVzdGVk",
            false,
        );
        assert!(env.unavailable_reason.is_none());
        assert_eq!(env.platform_label.as_deref(), Some("ubuntu-amd64"));
    }

    #[test]
    fn missing_key_disables_updates() {
        for key in ["", "  ", UNCONFIGURED_PUBKEY] {
            let env =
                UpdaterEnvironment::new("0.1.0", Some(LINUX), Some(Installer::Deb), key, false);
            assert!(!env.pubkey_configured);
            assert!(env.unavailable_reason.unwrap().contains("signing key"));
        }
    }

    #[test]
    fn dev_and_unbundled_builds_do_not_update() {
        let dev = UpdaterEnvironment::new("0.1.0", Some(LINUX), Some(Installer::Deb), "k", true);
        assert!(dev.unavailable_reason.unwrap().contains("development"));
        let raw = UpdaterEnvironment::new("0.1.0", Some(LINUX), None, "k", false);
        assert!(raw.unavailable_reason.unwrap().contains("installer"));
        let unknown = UpdaterEnvironment::new("0.1.0", None, Some(Installer::Deb), "k", false);
        assert!(
            unknown
                .unavailable_reason
                .unwrap()
                .contains("not supported")
        );
    }

    #[test]
    fn cancel_without_download_is_a_noop() {
        assert!(!cancel(&UpdaterState::default()));
    }

    #[test]
    fn install_requires_check_and_download() {
        let state = UpdaterState::default();
        let err = install(&state).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
    }
}
