//! Supported operating systems, architectures and installer kinds, and the
//! naming conventions that tie them to release artifacts.

use std::fmt;

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Os {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Arch {
    X86_64,
    X86,
    Aarch64,
}

/// An operating system + CPU architecture pair that Elyvon ships builds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

/// The kind of installed package. Determines which artifact an update uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Installer {
    Nsis,
    Msi,
    /// macOS `.app` bundle (installed from the `.dmg`).
    App,
    Deb,
    AppImage,
}

/// A file attached to a release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArtifactKind {
    NsisSetup,
    Msi,
    Dmg,
    /// `.app.tar.gz` — the macOS updater payload.
    AppArchive,
    Deb,
    AppImage,
}

impl Os {
    /// Name used by the Tauri updater manifest (`darwin` for macOS).
    pub fn updater_name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Macos => "darwin",
            Os::Linux => "linux",
        }
    }
}

impl Arch {
    pub fn updater_name(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::X86 => "i686",
            Arch::Aarch64 => "aarch64",
        }
    }
}

impl Installer {
    pub fn updater_name(self) -> &'static str {
        match self {
            Installer::Nsis => "nsis",
            Installer::Msi => "msi",
            Installer::App => "app",
            Installer::Deb => "deb",
            Installer::AppImage => "appimage",
        }
    }

    pub fn from_updater_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.updater_name() == s)
    }

    pub const ALL: [Installer; 5] = [
        Installer::Nsis,
        Installer::Msi,
        Installer::App,
        Installer::Deb,
        Installer::AppImage,
    ];

    pub fn os(self) -> Os {
        match self {
            Installer::Nsis | Installer::Msi => Os::Windows,
            Installer::App => Os::Macos,
            Installer::Deb | Installer::AppImage => Os::Linux,
        }
    }

    /// The artifact the updater downloads for this installer kind.
    pub fn update_artifact(self) -> ArtifactKind {
        match self {
            Installer::Nsis => ArtifactKind::NsisSetup,
            Installer::Msi => ArtifactKind::Msi,
            Installer::App => ArtifactKind::AppArchive,
            Installer::Deb => ArtifactKind::Deb,
            Installer::AppImage => ArtifactKind::AppImage,
        }
    }
}

impl ArtifactKind {
    pub fn suffix(self) -> &'static str {
        match self {
            ArtifactKind::NsisSetup => "-setup.exe",
            ArtifactKind::Msi => ".msi",
            ArtifactKind::Dmg => ".dmg",
            ArtifactKind::AppArchive => ".app.tar.gz",
            ArtifactKind::Deb => ".deb",
            ArtifactKind::AppImage => ".AppImage",
        }
    }

    pub fn from_cli_name(s: &str) -> Option<Self> {
        Some(match s {
            "nsis" => ArtifactKind::NsisSetup,
            "msi" => ArtifactKind::Msi,
            "dmg" => ArtifactKind::Dmg,
            "app" => ArtifactKind::AppArchive,
            "deb" => ArtifactKind::Deb,
            "appimage" => ArtifactKind::AppImage,
            _ => return None,
        })
    }
}

impl Platform {
    /// The seven platforms Elyvon Studio is built for, in release order.
    pub const SUPPORTED: [Platform; 7] = [
        Platform {
            os: Os::Windows,
            arch: Arch::X86_64,
        },
        Platform {
            os: Os::Windows,
            arch: Arch::X86,
        },
        Platform {
            os: Os::Windows,
            arch: Arch::Aarch64,
        },
        Platform {
            os: Os::Macos,
            arch: Arch::X86_64,
        },
        Platform {
            os: Os::Macos,
            arch: Arch::Aarch64,
        },
        Platform {
            os: Os::Linux,
            arch: Arch::X86_64,
        },
        Platform {
            os: Os::Linux,
            arch: Arch::Aarch64,
        },
    ];

    /// The platform this binary was compiled for (not the host it runs on:
    /// an x86 build under emulation must keep receiving x86 updates).
    pub fn current() -> Option<Self> {
        let os = if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            return None;
        };
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "x86") {
            Arch::X86
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else {
            return None;
        };
        Some(Self { os, arch }).filter(Self::is_supported)
    }

    pub fn is_supported(&self) -> bool {
        Self::SUPPORTED.contains(self)
    }

    pub fn rust_target(self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Windows, Arch::X86_64) => "x86_64-pc-windows-msvc",
            (Os::Windows, Arch::X86) => "i686-pc-windows-msvc",
            (Os::Windows, Arch::Aarch64) => "aarch64-pc-windows-msvc",
            (Os::Macos, Arch::X86_64) => "x86_64-apple-darwin",
            (Os::Macos, Arch::Aarch64) => "aarch64-apple-darwin",
            (Os::Linux, Arch::X86_64) => "x86_64-unknown-linux-gnu",
            (Os::Linux, Arch::Aarch64) => "aarch64-unknown-linux-gnu",
            (Os::Macos | Os::Linux, Arch::X86) => "unsupported",
        }
    }

    pub fn from_rust_target(triple: &str) -> Option<Self> {
        Self::SUPPORTED
            .into_iter()
            .find(|p| p.rust_target() == triple)
    }

    /// Human-readable label used in artifact file names.
    pub fn release_label(self) -> &'static str {
        match (self.os, self.arch) {
            (Os::Windows, Arch::X86_64) => "windows-x64",
            (Os::Windows, Arch::X86) => "windows-x86",
            (Os::Windows, Arch::Aarch64) => "windows-arm64",
            (Os::Macos, Arch::X86_64) => "macos-intel",
            (Os::Macos, Arch::Aarch64) => "macos-arm64",
            (Os::Linux, Arch::X86_64) => "ubuntu-amd64",
            (Os::Linux, Arch::Aarch64) => "ubuntu-arm64",
            (Os::Macos | Os::Linux, Arch::X86) => "unsupported",
        }
    }

    /// `{os}-{arch}` as used by the Tauri updater.
    pub fn updater_target(self) -> String {
        format!("{}-{}", self.os.updater_name(), self.arch.updater_name())
    }

    /// `{os}-{arch}-{installer}` — the manifest key Elyvon publishes.
    pub fn updater_key(self, installer: Installer) -> String {
        format!("{}-{}", self.updater_target(), installer.updater_name())
    }

    /// Parses a manifest key back into a platform and installer.
    pub fn parse_updater_key(key: &str) -> Option<(Platform, Option<Installer>)> {
        let mut parts = key.split('-');
        let os = parts.next()?;
        let arch = parts.next()?;
        let installer = parts.next();
        if parts.next().is_some() {
            return None;
        }
        let platform = Self::SUPPORTED
            .into_iter()
            .find(|p| p.os.updater_name() == os && p.arch.updater_name() == arch)?;
        match installer {
            None => Some((platform, None)),
            Some(name) => {
                let inst = Installer::from_updater_name(name)?;
                platform
                    .supports_installer(inst)
                    .then_some((platform, Some(inst)))
            }
        }
    }

    /// Installer kinds that can be updated in place on this platform.
    pub fn supports_installer(self, installer: Installer) -> bool {
        installer.os() == self.os && !(installer == Installer::Msi && self.arch == Arch::Aarch64)
    }

    /// Release artifacts produced for this platform.
    pub fn artifact_kinds(self) -> &'static [ArtifactKind] {
        match (self.os, self.arch) {
            (Os::Windows, Arch::Aarch64) => &[ArtifactKind::NsisSetup],
            (Os::Windows, _) => &[ArtifactKind::NsisSetup, ArtifactKind::Msi],
            (Os::Macos, _) => &[ArtifactKind::Dmg, ArtifactKind::AppArchive],
            (Os::Linux, _) => &[ArtifactKind::Deb, ArtifactKind::AppImage],
        }
    }

    /// Installers whose updates are published for this platform.
    pub fn update_installers(self) -> &'static [Installer] {
        match (self.os, self.arch) {
            (Os::Windows, Arch::Aarch64) => &[Installer::Nsis],
            (Os::Windows, _) => &[Installer::Nsis, Installer::Msi],
            (Os::Macos, _) => &[Installer::App],
            (Os::Linux, _) => &[Installer::AppImage, Installer::Deb],
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.release_label())
    }
}

/// File naming convention for release artifacts of one product.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactNaming {
    /// e.g. `Elyvon-Studio`.
    pub product: &'static str,
}

pub const STUDIO_NAMING: ArtifactNaming = ArtifactNaming {
    product: "Elyvon-Studio",
};

impl ArtifactNaming {
    /// e.g. `Elyvon-Studio-v0.1.0-windows-x64-setup.exe`. `version` has no `v`.
    pub fn file_name(&self, version: &str, platform: Platform, kind: ArtifactKind) -> String {
        format!(
            "{}-v{}-{}{}",
            self.product,
            version,
            platform.release_label(),
            kind.suffix()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_targets_round_trip() {
        for p in Platform::SUPPORTED {
            assert_eq!(Platform::from_rust_target(p.rust_target()), Some(p));
        }
        assert_eq!(
            Platform::from_rust_target("riscv64gc-unknown-linux-gnu"),
            None
        );
    }

    #[test]
    fn current_platform_is_supported_on_ci_hosts() {
        // Every CI host builds for one of the supported platforms.
        let p = Platform::current().expect("supported host");
        assert!(p.is_supported());
    }

    #[test]
    fn updater_keys_round_trip() {
        for p in Platform::SUPPORTED {
            for &i in p.update_installers() {
                let key = p.updater_key(i);
                assert_eq!(
                    Platform::parse_updater_key(&key),
                    Some((p, Some(i))),
                    "{key}"
                );
            }
            assert_eq!(
                Platform::parse_updater_key(&p.updater_target()),
                Some((p, None))
            );
        }
    }

    #[test]
    fn rejects_mismatched_or_unknown_keys() {
        for bad in [
            "windows-x86_64-deb",
            "darwin-x86_64-nsis",
            "linux-i686",
            "windows-aarch64-msi",
            "freebsd-x86_64",
            "windows-x86_64-nsis-extra",
            "windows",
        ] {
            assert_eq!(Platform::parse_updater_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn artifact_names_follow_convention() {
        let win = Platform::from_rust_target("x86_64-pc-windows-msvc").unwrap();
        let mac = Platform::from_rust_target("aarch64-apple-darwin").unwrap();
        let deb = Platform::from_rust_target("x86_64-unknown-linux-gnu").unwrap();
        let n = STUDIO_NAMING;
        assert_eq!(
            n.file_name("0.1.0", win, ArtifactKind::NsisSetup),
            "Elyvon-Studio-v0.1.0-windows-x64-setup.exe"
        );
        assert_eq!(
            n.file_name("0.1.0", mac, ArtifactKind::Dmg),
            "Elyvon-Studio-v0.1.0-macos-arm64.dmg"
        );
        assert_eq!(
            n.file_name("0.1.0", deb, ArtifactKind::Deb),
            "Elyvon-Studio-v0.1.0-ubuntu-amd64.deb"
        );
    }

    #[test]
    fn arm64_windows_has_no_msi() {
        let p = Platform::from_rust_target("aarch64-pc-windows-msvc").unwrap();
        assert!(!p.supports_installer(Installer::Msi));
        assert!(!p.artifact_kinds().contains(&ArtifactKind::Msi));
    }
}
