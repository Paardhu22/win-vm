//! OVMF/edk2 UEFI firmware discovery.
//!
//! Modern Windows boots via UEFI, so a VM needs an OVMF firmware pair: a
//! read-only `CODE` image and a writable `VARS` template that each VM gets its
//! own copy of. Distributions disagree about where these live and what they are
//! called, so the search is a preference-ordered table rather than one path.
//! Extending cross-distribution support means adding rows here.

use std::path::PathBuf;

use serde::Serialize;

use super::{Package, PackageManager, Requirement, Status, Sysroot};

struct Candidate {
    code: &'static str,
    vars: &'static str,
    secure_boot: bool,
    flash_size_mb: u32,
    origin: &'static str,
}

/// Known firmware locations, most preferred first.
///
/// Preference order is: Secure Boot capable before plain, and 4 MB flash before
/// the legacy 2 MB layout. Windows 11 requires Secure Boot, and the 4 MB build
/// is what current distributions ship.
const CANDIDATES: &[Candidate] = &[
    // Arch Linux (edk2-ovmf)
    Candidate {
        code: "/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd",
        vars: "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        secure_boot: true,
        flash_size_mb: 4,
        origin: "Arch (edk2-ovmf)",
    },
    Candidate {
        code: "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
        vars: "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        secure_boot: false,
        flash_size_mb: 4,
        origin: "Arch (edk2-ovmf)",
    },
    // Debian / Ubuntu (ovmf)
    Candidate {
        code: "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd",
        vars: "/usr/share/OVMF/OVMF_VARS_4M.ms.fd",
        secure_boot: true,
        flash_size_mb: 4,
        origin: "Debian/Ubuntu (ovmf)",
    },
    Candidate {
        code: "/usr/share/OVMF/OVMF_CODE_4M.fd",
        vars: "/usr/share/OVMF/OVMF_VARS_4M.fd",
        secure_boot: false,
        flash_size_mb: 4,
        origin: "Debian/Ubuntu (ovmf)",
    },
    // Fedora / RHEL (edk2-ovmf)
    Candidate {
        code: "/usr/share/edk2/ovmf/OVMF_CODE.secboot.fd",
        vars: "/usr/share/edk2/ovmf/OVMF_VARS.secboot.fd",
        secure_boot: true,
        flash_size_mb: 4,
        origin: "Fedora/RHEL (edk2-ovmf)",
    },
    Candidate {
        code: "/usr/share/edk2/ovmf/OVMF_CODE.fd",
        vars: "/usr/share/edk2/ovmf/OVMF_VARS.fd",
        secure_boot: false,
        flash_size_mb: 4,
        origin: "Fedora/RHEL (edk2-ovmf)",
    },
    // openSUSE (qemu-ovmf-x86_64)
    Candidate {
        code: "/usr/share/qemu/ovmf-x86_64-smm-code.bin",
        vars: "/usr/share/qemu/ovmf-x86_64-smm-vars.bin",
        secure_boot: true,
        flash_size_mb: 4,
        origin: "openSUSE (qemu-ovmf-x86_64)",
    },
    Candidate {
        code: "/usr/share/qemu/ovmf-x86_64-code.bin",
        vars: "/usr/share/qemu/ovmf-x86_64-vars.bin",
        secure_boot: false,
        flash_size_mb: 4,
        origin: "openSUSE (qemu-ovmf-x86_64)",
    },
    // Legacy 2 MB layouts, kept last as a fallback for older installs.
    Candidate {
        code: "/usr/share/edk2-ovmf/x64/OVMF_CODE.secboot.fd",
        vars: "/usr/share/edk2-ovmf/x64/OVMF_VARS.fd",
        secure_boot: true,
        flash_size_mb: 2,
        origin: "Arch (legacy edk2-ovmf)",
    },
    Candidate {
        code: "/usr/share/OVMF/OVMF_CODE.fd",
        vars: "/usr/share/OVMF/OVMF_VARS.fd",
        secure_boot: false,
        flash_size_mb: 2,
        origin: "Debian/Ubuntu (legacy ovmf)",
    },
];

/// A usable `CODE` + `VARS` firmware pair on this host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirmwarePair {
    /// Read-only firmware image, attached as the first pflash unit.
    pub code: PathBuf,
    /// Variable store *template*. Each VM gets a private copy; this file is
    /// never written to.
    pub vars_template: PathBuf,
    pub secure_boot: bool,
    pub flash_size_mb: u32,
    /// Which distribution's packaging this pair came from, for display.
    pub origin: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Firmware {
    /// Every pair found, in preference order.
    pub found: Vec<FirmwarePair>,
}

impl Firmware {
    pub fn detect_in(root: &Sysroot) -> Self {
        let found = CANDIDATES
            .iter()
            .filter(|c| root.exists(c.code) && root.exists(c.vars))
            .map(|c| FirmwarePair {
                code: root.resolve(c.code),
                vars_template: root.resolve(c.vars),
                secure_boot: c.secure_boot,
                flash_size_mb: c.flash_size_mb,
                origin: c.origin,
            })
            .collect();
        Firmware { found }
    }

    /// The pair DA-HOLY-VM would use, i.e. the most preferred one present.
    pub fn best(&self) -> Option<&FirmwarePair> {
        self.found.first()
    }

    pub fn secure_boot_capable(&self) -> bool {
        self.found.iter().any(|pair| pair.secure_boot)
    }

    pub(crate) fn requirement(&self, pm: PackageManager) -> Requirement {
        const ID: &str = "firmware.ovmf";
        const TITLE: &str = "UEFI firmware (OVMF)";

        let Some(best) = self.best() else {
            return Requirement::new(
                ID,
                TITLE,
                Status::Missing,
                "no OVMF firmware pair found in any known location",
            )
            .with_remedy(pm.install_hint(Package::Ovmf));
        };

        let detail = format!(
            "{} MB {} pair from {} at {}",
            best.flash_size_mb,
            if best.secure_boot {
                "Secure Boot capable"
            } else {
                "non Secure Boot"
            },
            best.origin,
            best.code.display()
        );

        if best.secure_boot {
            Requirement::new(ID, TITLE, Status::Ok, detail)
        } else {
            Requirement::new(ID, TITLE, Status::Warn, detail).with_remedy(
                "this firmware cannot do Secure Boot, which Windows 11 requires; Windows 10 guests \
                 are unaffected. Installing the full OVMF package usually provides a `secboot` \
                 variant alongside it",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Build a fixture sysroot containing the given absolute paths.
    fn fixture(name: &str, files: &[&str]) -> (Sysroot, PathBuf) {
        let root = std::env::temp_dir().join(format!("daholyvm-fw-{name}"));
        let _ = fs::remove_dir_all(&root);
        for file in files {
            let path = root.join(file.trim_start_matches('/'));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"fixture").unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        (Sysroot::at(&root), root)
    }

    #[test]
    fn finds_nothing_on_a_bare_root() {
        let (root, _dir) = fixture("bare", &[]);
        let fw = Firmware::detect_in(&root);
        assert!(fw.best().is_none());
        let req = fw.requirement(PackageManager::Pacman);
        assert_eq!(req.status, Status::Missing);
        assert_eq!(
            req.remedy.as_deref(),
            Some("sudo pacman -S --needed edk2-ovmf")
        );
    }

    #[test]
    fn prefers_secure_boot_over_plain_when_both_are_present() {
        let (root, dir) = fixture(
            "arch",
            &[
                "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                "/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd",
                "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
            ],
        );
        let fw = Firmware::detect_in(&root);
        assert_eq!(fw.found.len(), 2);
        let best = fw.best().unwrap();
        assert!(best.secure_boot);
        assert_eq!(
            best.code,
            dir.join("usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd")
        );
        assert_eq!(fw.requirement(PackageManager::Pacman).status, Status::Ok);
    }

    #[test]
    fn a_code_image_without_its_vars_template_is_not_usable() {
        let (root, _dir) = fixture("halfpair", &["/usr/share/OVMF/OVMF_CODE_4M.fd"]);
        assert!(Firmware::detect_in(&root).best().is_none());
    }

    #[test]
    fn non_secure_boot_only_warns_about_windows_11() {
        let (root, _dir) = fixture(
            "debian",
            &[
                "/usr/share/OVMF/OVMF_CODE_4M.fd",
                "/usr/share/OVMF/OVMF_VARS_4M.fd",
            ],
        );
        let fw = Firmware::detect_in(&root);
        assert!(!fw.secure_boot_capable());
        let req = fw.requirement(PackageManager::Apt);
        assert_eq!(req.status, Status::Warn);
        assert!(req.remedy.unwrap().contains("Windows 11"));
    }

    #[test]
    fn every_candidate_path_is_absolute() {
        // The Sysroot rebasing contract depends on this.
        for candidate in CANDIDATES {
            assert!(
                Path::new(candidate.code).is_absolute(),
                "{}",
                candidate.code
            );
            assert!(
                Path::new(candidate.vars).is_absolute(),
                "{}",
                candidate.vars
            );
        }
    }
}
