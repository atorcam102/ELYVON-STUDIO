//! The updater manifest (`latest.json`) published with every release.
//!
//! The format is the "static JSON" format understood by the Tauri updater
//! plugin. Elyvon publishes one entry per `{os}-{arch}-{installer}` key and
//! never a bare `{os}-{arch}` fallback, so an installation can only ever
//! resolve to the artifact built for its own platform, architecture and
//! installer kind.
//!
//! [`UpdateManifest::parse`] is deliberately stricter than the updater: the
//! release pipeline uses it to refuse publishing a manifest that points at the
//! wrong file, an insecure URL or a malformed signature.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::platform::{ArtifactNaming, Installer, Platform};

pub const MAX_NOTES_CHARS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<String>,
    pub platforms: BTreeMap<String, PlatformAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformAsset {
    /// Base64 minisign signature produced by `tauri signer`.
    pub signature: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest is not valid JSON: {0}")]
    Json(String),
    #[error("invalid version {0:?}: must be SemVer without a leading 'v'")]
    InvalidVersion(String),
    #[error("manifest lists no platforms")]
    NoPlatforms,
    #[error("unknown or unsupported platform key {0:?}")]
    UnknownPlatformKey(String),
    #[error("platform key {0:?} must name an installer ({{os}}-{{arch}}-{{installer}})")]
    MissingInstaller(String),
    #[error("{key}: URL must use https: {url}")]
    InsecureUrl { key: String, url: String },
    #[error("{key}: URL points to {found:?}, expected {expected:?}")]
    WrongArtifact {
        key: String,
        expected: String,
        found: String,
    },
    #[error("{key}: invalid signature: {reason}")]
    BadSignature { key: String, reason: String },
    #[error("release notes are longer than {MAX_NOTES_CHARS} characters")]
    NotesTooLong,
    #[error("missing platforms: {}", .0.join(", "))]
    MissingPlatforms(Vec<String>),
    #[error("no update is published for {0}")]
    NoAssetForPlatform(String),
}

/// Last path segment of a URL, ignoring query and fragment.
fn url_file_name(url: &str) -> &str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path)
}

/// Checks that a minisign signature (as produced by `tauri signer sign`) is well formed.
pub fn check_signature(sig: &str) -> Result<(), String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(sig.trim())
        .map_err(|e| format!("not base64: {e}"))?;
    let text = String::from_utf8(decoded).map_err(|_| "not UTF-8 text".to_owned())?;
    let mut lines = text.lines();
    if !lines
        .next()
        .is_some_and(|l| l.starts_with("untrusted comment:"))
    {
        return Err("missing untrusted comment".into());
    }
    let sig_line = lines.next().ok_or("missing signature line")?;
    base64::engine::general_purpose::STANDARD
        .decode(sig_line.trim())
        .map_err(|_| "signature line is not base64".to_owned())?;
    if !lines
        .next()
        .is_some_and(|l| l.starts_with("trusted comment:"))
    {
        return Err("missing trusted comment".into());
    }
    if lines.next().is_none_or(|l| l.trim().is_empty()) {
        return Err("missing global signature".into());
    }
    Ok(())
}

/// Cryptographically verifies `data` against a Tauri/minisign signature.
///
/// `pubkey_b64` is the `plugins.updater.pubkey` value embedded in the
/// application; `sig_b64` the content of the `.sig` file. When
/// `expected_version` is given, the signature's trusted comment must carry
/// exactly that `version:` (what the updater's `requireSignedVersion` checks).
pub fn verify_signature(
    pubkey_b64: &str,
    data: &[u8],
    sig_b64: &str,
    expected_version: Option<&str>,
) -> Result<(), String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let decode = |what: &str, v: &str| {
        b64.decode(v.trim())
            .map_err(|e| format!("{what} is not base64: {e}"))
            .and_then(|bytes| String::from_utf8(bytes).map_err(|_| format!("{what} is not UTF-8")))
    };
    let key = minisign_verify::PublicKey::decode(&decode("public key", pubkey_b64)?)
        .map_err(|e| format!("invalid public key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&decode("signature", sig_b64)?)
        .map_err(|e| format!("invalid signature: {e}"))?;
    key.verify(data, &sig, true)
        .map_err(|e| format!("signature does not match the public key: {e}"))?;
    if let Some(expected) = expected_version {
        let signed = sig
            .trusted_comment()
            .split('\t')
            .find_map(|f| f.strip_prefix("version:"))
            .ok_or("signature is not bound to a version (sign with --app-version)")?;
        if signed.trim_start_matches('v') != expected.trim_start_matches('v') {
            return Err(format!(
                "signature is for version {signed}, expected {expected}"
            ));
        }
    }
    Ok(())
}

/// Verifies that `url` downloads exactly the artifact built for `platform`
/// and `installer` at `version`. Used by the application before installing.
pub fn check_artifact_url(
    naming: &ArtifactNaming,
    version: &str,
    platform: Platform,
    installer: Installer,
    key: &str,
    url: &str,
) -> Result<(), ManifestError> {
    if !url.starts_with("https://") {
        return Err(ManifestError::InsecureUrl {
            key: key.into(),
            url: url.into(),
        });
    }
    let expected = naming.file_name(version, platform, installer.update_artifact());
    let found = url_file_name(url);
    if found != expected {
        return Err(ManifestError::WrongArtifact {
            key: key.into(),
            expected,
            found: found.into(),
        });
    }
    Ok(())
}

impl UpdateManifest {
    /// Parses and fully validates a manifest.
    pub fn parse(bytes: &[u8], naming: &ArtifactNaming) -> Result<Self, ManifestError> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::Json(e.to_string()))?;
        manifest.validate(naming)?;
        Ok(manifest)
    }

    pub fn parsed_version(&self) -> Result<Version, ManifestError> {
        Version::parse(&self.version)
            .map_err(|_| ManifestError::InvalidVersion(self.version.clone()))
    }

    pub fn validate(&self, naming: &ArtifactNaming) -> Result<(), ManifestError> {
        self.parsed_version()?;
        if self
            .notes
            .as_ref()
            .is_some_and(|n| n.chars().count() > MAX_NOTES_CHARS)
        {
            return Err(ManifestError::NotesTooLong);
        }
        if self.platforms.is_empty() {
            return Err(ManifestError::NoPlatforms);
        }
        for (key, asset) in &self.platforms {
            let (platform, installer) = Platform::parse_updater_key(key)
                .ok_or_else(|| ManifestError::UnknownPlatformKey(key.clone()))?;
            let installer =
                installer.ok_or_else(|| ManifestError::MissingInstaller(key.clone()))?;
            check_artifact_url(naming, &self.version, platform, installer, key, &asset.url)?;
            check_signature(&asset.signature).map_err(|reason| ManifestError::BadSignature {
                key: key.clone(),
                reason,
            })?;
        }
        Ok(())
    }

    /// The asset for exactly this platform and installer — never a fallback.
    pub fn select(
        &self,
        platform: Platform,
        installer: Installer,
    ) -> Result<&PlatformAsset, ManifestError> {
        let key = platform.updater_key(installer);
        self.platforms
            .get(&key)
            .ok_or(ManifestError::NoAssetForPlatform(key))
    }

    /// Every key a complete release must contain.
    pub fn complete_key_set() -> Vec<String> {
        Platform::SUPPORTED
            .iter()
            .flat_map(|p| p.update_installers().iter().map(|i| p.updater_key(*i)))
            .collect()
    }

    pub fn missing_keys(&self, required: &[String]) -> Vec<String> {
        required
            .iter()
            .filter(|k| !self.platforms.contains_key(*k))
            .cloned()
            .collect()
    }

    /// Builds a manifest from the signed updater artifacts found in `dir`.
    ///
    /// Artifacts must already carry their canonical names (see
    /// [`ArtifactNaming::file_name`]) with a sibling `.sig` file.
    pub fn assemble(
        dir: &Path,
        naming: &ArtifactNaming,
        version: &str,
        base_url: &str,
        notes: Option<String>,
        pub_date: Option<String>,
    ) -> Result<Self, AssembleError> {
        let base_url = base_url.trim_end_matches('/');
        let mut platforms = BTreeMap::new();
        for platform in Platform::SUPPORTED {
            for &installer in platform.update_installers() {
                let file = naming.file_name(version, platform, installer.update_artifact());
                let sig_path = dir.join(format!("{file}.sig"));
                if !dir.join(&file).is_file() || !sig_path.is_file() {
                    continue;
                }
                let signature = std::fs::read_to_string(&sig_path)
                    .map_err(|e| AssembleError::Io(format!("{}: {e}", sig_path.display())))?
                    .trim()
                    .to_owned();
                platforms.insert(
                    platform.updater_key(installer),
                    PlatformAsset {
                        signature,
                        url: format!("{base_url}/{file}"),
                    },
                );
            }
        }
        let manifest = Self {
            version: version.to_owned(),
            notes,
            pub_date,
            platforms,
        };
        manifest.validate(naming)?;
        Ok(manifest)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssembleError {
    #[error("{0}")]
    Io(String),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{ArtifactKind, STUDIO_NAMING};

    /// A syntactically valid minisign signature (random bytes, not a real one).
    fn fake_signature() -> String {
        let b64 = base64::engine::general_purpose::STANDARD;
        let text = format!(
            "untrusted comment: signature from tauri secret key\n{}\ntrusted comment: timestamp:1700000000\tfile:x\n{}\n",
            b64.encode([7u8; 74]),
            b64.encode([9u8; 64])
        );
        b64.encode(text)
    }

    fn url(platform: &str, kind: ArtifactKind) -> String {
        let p = Platform::from_rust_target(platform).unwrap();
        format!(
            "https://github.com/atorcam102/Elyvon/releases/download/v0.2.0/{}",
            STUDIO_NAMING.file_name("0.2.0", p, kind)
        )
    }

    fn manifest_json(key: &str, url: &str, sig: &str) -> String {
        format!(
            r#"{{"version":"0.2.0","notes":"Fixes","pub_date":"2026-10-15T12:00:00Z","platforms":{{"{key}":{{"signature":"{sig}","url":"{url}"}}}}}}"#
        )
    }

    #[test]
    fn accepts_valid_manifest_and_selects_exact_platform() {
        let json = manifest_json(
            "windows-x86_64-nsis",
            &url("x86_64-pc-windows-msvc", ArtifactKind::NsisSetup),
            &fake_signature(),
        );
        let m = UpdateManifest::parse(json.as_bytes(), &STUDIO_NAMING).unwrap();
        let x64 = Platform::from_rust_target("x86_64-pc-windows-msvc").unwrap();
        let arm = Platform::from_rust_target("aarch64-pc-windows-msvc").unwrap();
        assert!(m.select(x64, Installer::Nsis).is_ok());
        assert!(matches!(
            m.select(arm, Installer::Nsis),
            Err(ManifestError::NoAssetForPlatform(_))
        ));
        assert!(m.select(x64, Installer::Msi).is_err());
    }

    #[test]
    fn rejects_artifact_for_another_architecture() {
        // arm64 key pointing at the x64 installer.
        let json = manifest_json(
            "windows-aarch64-nsis",
            &url("x86_64-pc-windows-msvc", ArtifactKind::NsisSetup),
            &fake_signature(),
        );
        let err = UpdateManifest::parse(json.as_bytes(), &STUDIO_NAMING).unwrap_err();
        assert!(matches!(err, ManifestError::WrongArtifact { .. }), "{err}");
    }

    #[test]
    fn rejects_invalid_metadata() {
        let good_url = url("aarch64-apple-darwin", ArtifactKind::AppArchive);
        let sig = fake_signature();
        let cases = [
            ("not json", ManifestError::Json(String::new())),
            (
                r#"{"version":"v0.2.0","platforms":{}}"#,
                ManifestError::InvalidVersion(String::new()),
            ),
            (
                r#"{"version":"0.2.0","platforms":{}}"#,
                ManifestError::NoPlatforms,
            ),
        ];
        for (json, expected) in cases {
            let err = UpdateManifest::parse(json.as_bytes(), &STUDIO_NAMING).unwrap_err();
            assert_eq!(
                std::mem::discriminant(&err),
                std::mem::discriminant(&expected),
                "{json}"
            );
        }

        let bad = |key: &str, url: &str, sig: &str| {
            UpdateManifest::parse(manifest_json(key, url, sig).as_bytes(), &STUDIO_NAMING)
                .unwrap_err()
        };
        assert!(matches!(
            bad("darwin-aarch64", &good_url, &sig),
            ManifestError::MissingInstaller(_)
        ));
        assert!(matches!(
            bad("darwin-aarch64-nsis", &good_url, &sig),
            ManifestError::UnknownPlatformKey(_)
        ));
        assert!(matches!(
            bad("solaris-sparc-app", &good_url, &sig),
            ManifestError::UnknownPlatformKey(_)
        ));
        let http = good_url.replace("https://", "http://");
        assert!(matches!(
            bad("darwin-aarch64-app", &http, &sig),
            ManifestError::InsecureUrl { .. }
        ));
        assert!(matches!(
            bad("darwin-aarch64-app", &good_url, ""),
            ManifestError::BadSignature { .. }
        ));
        assert!(matches!(
            bad("darwin-aarch64-app", &good_url, "!!!"),
            ManifestError::BadSignature { .. }
        ));
        let other_version = good_url.replace("v0.2.0-macos", "v0.1.0-macos");
        assert!(matches!(
            bad("darwin-aarch64-app", &other_version, &sig),
            ManifestError::WrongArtifact { .. }
        ));
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/test-signing")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn verifies_real_signatures_and_versions() {
        let key = String::from_utf8(fixture("test-only.pub.b64")).unwrap();
        let data = fixture("payload.bin");
        let sig = String::from_utf8(fixture("payload.bin.sig")).unwrap();
        verify_signature(&key, &data, &sig, Some("0.2.0")).unwrap();
        verify_signature(&key, &data, &sig, None).unwrap();
        assert!(
            verify_signature(&key, &data, &sig, Some("0.3.0"))
                .unwrap_err()
                .contains("0.2.0")
        );
        // Tampered payload.
        let mut bad = data.clone();
        bad[0] ^= 1;
        assert!(verify_signature(&key, &bad, &sig, None).is_err());
        // Unversioned signatures are valid but fail the version binding.
        let unversioned = String::from_utf8(fixture("payload-unversioned.bin.sig")).unwrap();
        verify_signature(&key, &data, &unversioned, None).unwrap();
        assert!(verify_signature(&key, &data, &unversioned, Some("0.2.0")).is_err());
        // Wrong or placeholder key.
        assert!(verify_signature("UNCONFIGURED", &data, &sig, None).is_err());
    }

    #[test]
    fn real_signatures_pass_structure_check() {
        let sig = String::from_utf8(fixture("payload.bin.sig")).unwrap();
        check_signature(&sig).unwrap();
    }

    #[test]
    fn signature_structure_is_checked() {
        assert!(check_signature(&fake_signature()).is_ok());
        let b64 = base64::engine::general_purpose::STANDARD;
        assert!(check_signature(&b64.encode("hello")).is_err());
        assert!(check_signature(&b64.encode("untrusted comment: x\nAAAA\n")).is_err());
    }

    #[test]
    fn assemble_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let linux = Platform::from_rust_target("x86_64-unknown-linux-gnu").unwrap();
        for kind in [ArtifactKind::AppImage, ArtifactKind::Deb] {
            let name = STUDIO_NAMING.file_name("0.2.0", linux, kind);
            std::fs::write(dir.path().join(&name), b"bin").unwrap();
            std::fs::write(dir.path().join(format!("{name}.sig")), fake_signature()).unwrap();
        }
        // An unsigned artifact is not published to the updater.
        let mac = Platform::from_rust_target("aarch64-apple-darwin").unwrap();
        std::fs::write(
            dir.path()
                .join(STUDIO_NAMING.file_name("0.2.0", mac, ArtifactKind::AppArchive)),
            b"x",
        )
        .unwrap();

        let m = UpdateManifest::assemble(
            dir.path(),
            &STUDIO_NAMING,
            "0.2.0",
            "https://github.com/atorcam102/Elyvon/releases/download/v0.2.0/",
            Some("notes".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            m.platforms.keys().collect::<Vec<_>>(),
            ["linux-x86_64-appimage", "linux-x86_64-deb"]
        );
        let missing = m.missing_keys(&UpdateManifest::complete_key_set());
        assert_eq!(missing.len(), UpdateManifest::complete_key_set().len() - 2);
        // Round-trips through JSON and re-validates.
        let json = serde_json::to_vec(&m).unwrap();
        assert_eq!(UpdateManifest::parse(&json, &STUDIO_NAMING).unwrap(), m);
    }

    #[test]
    fn complete_key_set_covers_all_platforms() {
        let keys = UpdateManifest::complete_key_set();
        assert_eq!(keys.len(), 11);
        assert!(keys.contains(&"windows-i686-nsis".to_owned()));
        assert!(keys.contains(&"linux-aarch64-deb".to_owned()));
        assert!(!keys.contains(&"windows-aarch64-msi".to_owned()));
    }
}
