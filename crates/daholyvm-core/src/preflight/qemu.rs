//! Discovery of the QEMU binaries DA-HOLY-VM drives.
//!
//! Two binaries are needed: `qemu-system-x86_64` to run the guest and
//! `qemu-img` to create virtual disks. Merely finding a name on `PATH` is not
//! enough — unrelated SDKs (the Android SDK, for one) ship their own ancient
//! `qemu-img` and can shadow the system install — so every discovered binary is
//! version-checked and the resolved path is always reported back to the user.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use super::{which, Package, PackageManager, Requirement, Status};

pub const QEMU_SYSTEM_BINARY: &str = "qemu-system-x86_64";
pub const QEMU_IMG_BINARY: &str = "qemu-img";

/// Oldest QEMU that supports the machine types, UEFI pflash layout and device
/// models the MVP generates command lines for.
pub const MIN_QEMU_VERSION: Version = Version {
    major: 6,
    minor: 0,
    patch: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A QEMU binary located on `PATH`.
#[derive(Debug, Clone, Serialize)]
pub struct QemuBinary {
    pub path: PathBuf,
    /// `None` when `--version` output could not be parsed.
    pub version: Option<Version>,
}

impl QemuBinary {
    fn probe(name: &str) -> Option<Self> {
        let path = which::find(name)?;
        // Fixed argument vector, no shell: nothing user-supplied reaches here.
        let version = Command::new(&path)
            .arg("--version")
            .output()
            .ok()
            .and_then(|out| parse_version(&String::from_utf8_lossy(&out.stdout)));
        Some(QemuBinary { path, version })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Qemu {
    pub system: Option<QemuBinary>,
    pub img: Option<QemuBinary>,
}

impl Qemu {
    /// Probe `PATH`. Unlike the other checks this deliberately does not go
    /// through [`super::Sysroot`]: the user's real `PATH` is what matters, and
    /// it is exactly the thing we want to catch shadowing in.
    pub fn detect() -> Self {
        Qemu {
            system: QemuBinary::probe(QEMU_SYSTEM_BINARY),
            img: QemuBinary::probe(QEMU_IMG_BINARY),
        }
    }

    pub(crate) fn system_requirement(&self, pm: PackageManager) -> Requirement {
        self.binary_requirement(
            "qemu.system",
            "QEMU system emulator",
            QEMU_SYSTEM_BINARY,
            self.system.as_ref(),
            pm,
        )
    }

    pub(crate) fn img_requirement(&self, pm: PackageManager) -> Requirement {
        self.binary_requirement(
            "qemu.img",
            "QEMU disk image tool",
            QEMU_IMG_BINARY,
            self.img.as_ref(),
            pm,
        )
    }

    fn binary_requirement(
        &self,
        id: &'static str,
        title: &'static str,
        name: &'static str,
        binary: Option<&QemuBinary>,
        pm: PackageManager,
    ) -> Requirement {
        let Some(binary) = binary else {
            return Requirement::new(
                id,
                title,
                Status::Missing,
                format!("`{name}` was not found on PATH"),
            )
            .with_remedy(pm.install_hint(Package::Qemu));
        };

        let shown = binary.path.display();
        match binary.version {
            Some(version) if version >= MIN_QEMU_VERSION => {
                Requirement::new(id, title, Status::Ok, format!("{version} at {shown}"))
            }
            Some(version) => Requirement::new(
                id,
                title,
                Status::Warn,
                format!("{version} at {shown} is older than the required {MIN_QEMU_VERSION}"),
            )
            .with_remedy(format!(
                "another `{name}` earlier in PATH may be shadowing your system install \
                 (SDKs and toolchains often bundle their own); check `which -a {name}`, then \
                 install or prefer a current QEMU: {}",
                pm.install_hint(Package::Qemu)
            )),
            None => Requirement::new(
                id,
                title,
                Status::Warn,
                format!("found at {shown} but its version could not be determined"),
            )
            .with_remedy(format!("check that `{shown} --version` runs correctly")),
        }
    }
}

/// Extract a version from `--version` output.
///
/// Handles both `QEMU emulator version 9.1.0` and the noisier
/// `qemu-img version 2.12.0(v2.12.0-19097-g85fa07f04ef)`.
pub fn parse_version(text: &str) -> Option<Version> {
    text.lines()
        .next()?
        .split_whitespace()
        .find_map(version_from_token)
}

fn version_from_token(token: &str) -> Option<Version> {
    let numeric: String = token
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    // Require at least `major.minor` so bare numbers ("version 9") and unrelated
    // digits in a binary name are not mistaken for a version.
    if !numeric.contains('.') {
        return None;
    }
    let mut parts = numeric.split('.');
    Some(Version {
        major: parts.next()?.parse().ok()?,
        minor: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        patch: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qemu_system_version() {
        let out = "QEMU emulator version 9.1.0\nCopyright (c) 2003-2024 Fabrice Bellard\n";
        assert_eq!(
            parse_version(out),
            Some(Version {
                major: 9,
                minor: 1,
                patch: 0
            })
        );
    }

    #[test]
    fn parses_qemu_img_version_with_trailing_build_metadata() {
        let out = "qemu-img version 2.12.0(v2.12.0-19097-g85fa07f04ef)\n";
        assert_eq!(
            parse_version(out),
            Some(Version {
                major: 2,
                minor: 12,
                patch: 0
            })
        );
    }

    #[test]
    fn ignores_digits_in_the_binary_name() {
        // `x86_64` must not be read as a version.
        let out = "qemu-system-x86_64 version 8.2.2\n";
        assert_eq!(parse_version(out).unwrap().major, 8);
    }

    #[test]
    fn unparseable_output_is_none() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("no version here\n"), None);
    }

    #[test]
    fn versions_order_numerically_not_lexically() {
        let v = |major, minor| Version {
            major,
            minor,
            patch: 0,
        };
        assert!(!(v(9, 1) > v(10, 0)));
        assert!(v(10, 0) > v(9, 1), "10.0 must sort above 9.1");
        assert!(v(2, 12) < MIN_QEMU_VERSION);
        assert!(v(6, 0) >= MIN_QEMU_VERSION);
    }

    #[test]
    fn missing_binary_yields_an_install_command() {
        let qemu = Qemu {
            system: None,
            img: None,
        };
        let req = qemu.system_requirement(PackageManager::Pacman);
        assert_eq!(req.status, Status::Missing);
        assert_eq!(
            req.remedy.as_deref(),
            Some("sudo pacman -S --needed qemu-desktop")
        );
    }

    #[test]
    fn outdated_binary_warns_about_path_shadowing() {
        let qemu = Qemu {
            system: None,
            img: Some(QemuBinary {
                path: PathBuf::from("/home/user/Android/Sdk/emulator/qemu-img"),
                version: Some(Version {
                    major: 2,
                    minor: 12,
                    patch: 0,
                }),
            }),
        };
        let req = qemu.img_requirement(PackageManager::Pacman);
        assert_eq!(req.status, Status::Warn);
        assert!(
            req.detail.contains("Android/Sdk"),
            "the resolved path must be shown"
        );
        assert!(req.remedy.unwrap().contains("which -a"));
    }
}
