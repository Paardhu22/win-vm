//! Discovery of `swtpm`, the software TPM 2.0 emulator.
//!
//! Windows 11 checks for a TPM 2.0 during setup and refuses to install without
//! one, with an error message ("This PC can't run Windows 11") that says
//! nothing about which requirement failed. QEMU does not emulate a TPM itself;
//! it talks to an external emulator over a socket, and `swtpm` is that
//! emulator.
//!
//! Its absence is a **warning, not a blocker**. A VM still starts and Windows
//! 10 installs perfectly well without it, so refusing to launch would be wrong.
//! Only a guest that actually wants a TPM cares, and that is decided per VM.

use std::path::PathBuf;

use serde::Serialize;

use super::{which, Package, PackageManager, Requirement, Status};

pub const SWTPM_BINARY: &str = "swtpm";

#[derive(Debug, Clone, Serialize)]
pub struct Tpm {
    /// Resolved path to `swtpm`, reported so `PATH` shadowing is visible.
    pub swtpm: Option<PathBuf>,
}

impl Tpm {
    /// Probe `PATH`. Like the QEMU probe, this deliberately does not go through
    /// [`super::Sysroot`]: the user's real `PATH` is what will be used.
    pub fn detect() -> Self {
        Tpm {
            swtpm: which::find(SWTPM_BINARY),
        }
    }

    pub fn available(&self) -> bool {
        self.swtpm.is_some()
    }

    pub(crate) fn requirement(&self, pm: PackageManager) -> Requirement {
        const ID: &str = "tpm.swtpm";
        const TITLE: &str = "TPM 2.0 emulator (swtpm)";

        match &self.swtpm {
            Some(path) => Requirement::new(ID, TITLE, Status::Ok, format!("{}", path.display())),
            None => Requirement::new(
                ID,
                TITLE,
                Status::Warn,
                format!("`{SWTPM_BINARY}` was not found on PATH"),
            )
            .with_remedy(format!(
                "Windows 11 requires a TPM 2.0 and will refuse to install without one; \
                 Windows 10 guests are unaffected: {}",
                pm.install_hint(Package::Swtpm)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_emulator_warns_rather_than_blocks() {
        let req = Tpm { swtpm: None }.requirement(PackageManager::Pacman);
        assert_eq!(
            req.status,
            Status::Warn,
            "a VM without a TPM still starts, and Windows 10 does not care"
        );
        assert!(req
            .remedy
            .unwrap()
            .contains("sudo pacman -S --needed swtpm"));
    }

    #[test]
    fn a_present_emulator_is_reported_by_resolved_path() {
        let req = Tpm {
            swtpm: Some(PathBuf::from("/usr/bin/swtpm")),
        }
        .requirement(PackageManager::Pacman);
        assert_eq!(req.status, Status::Ok);
        assert!(req.detail.contains("/usr/bin/swtpm"));
        assert!(req.remedy.is_none());
    }

    #[test]
    fn the_remedy_names_windows_11_because_that_is_the_symptom() {
        let req = Tpm { swtpm: None }.requirement(PackageManager::Apt);
        // The user meets this as "This PC can't run Windows 11", so the remedy
        // has to connect the two for them.
        assert!(req.remedy.unwrap().contains("Windows 11"));
    }
}
