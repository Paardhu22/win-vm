//! Where DA-HOLY-VM keeps virtual machines on disk.
//!
//! One directory per VM under the XDG data directory, holding everything that
//! belongs to that guest:
//!
//! ```text
//! ~/.local/share/daholyvm/vms/win11/
//!     config.toml     the VmConfig, hand-editable
//!     disk.qcow2      the system disk
//!     OVMF_VARS.fd    this VM's private UEFI variable store
//! ```
//!
//! Grouping by VM rather than by file type means a VM can be backed up, copied
//! or deleted as one directory, and it is obvious to the user what belongs to
//! what.
//!
//! Like [`crate::preflight::Sysroot`], the root is injectable so the tests can
//! exercise the real layout inside a temporary directory.

use std::path::{Path, PathBuf};

use crate::config::VmName;
use crate::{Error, Result};

pub const CONFIG_FILE: &str = "config.toml";
pub const DISK_FILE: &str = "disk.qcow2";
pub const VARS_FILE: &str = "OVMF_VARS.fd";

const APP_DIR: &str = "daholyvm";
const VMS_DIR: &str = "vms";

/// The root of DA-HOLY-VM's storage.
#[derive(Debug, Clone)]
pub struct Paths {
    data: PathBuf,
}

impl Paths {
    /// Resolve from the environment, honouring `XDG_DATA_HOME`.
    pub fn from_env() -> Result<Self> {
        let data = match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            // The XDG spec's own fallback when the variable is unset or empty.
            _ => PathBuf::from(std::env::var_os("HOME").ok_or(Error::NoHome)?).join(".local/share"),
        };
        Ok(Paths {
            data: data.join(APP_DIR),
        })
    }

    /// Storage rooted at an explicit directory, used by the tests.
    pub fn at(data: impl Into<PathBuf>) -> Self {
        Paths { data: data.into() }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn vms_dir(&self) -> PathBuf {
        self.data.join(VMS_DIR)
    }

    /// The directory belonging to one VM. The name is already validated, so it
    /// cannot escape `vms/`.
    pub fn vm(&self, name: &VmName) -> VmPaths {
        VmPaths {
            dir: self.vms_dir().join(name.as_str()),
        }
    }

    /// Every VM on disk, in sorted order.
    ///
    /// A directory whose name is not a valid [`VmName`] is skipped rather than
    /// reported: DA-HOLY-VM did not create it, so it is not a VM.
    pub fn list(&self) -> Vec<VmName> {
        let Ok(entries) = std::fs::read_dir(self.vms_dir()) else {
            return Vec::new();
        };
        let mut names: Vec<VmName> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| VmName::new(entry.file_name().to_str()?).ok())
            .collect();
        names.sort();
        names
    }
}

/// The files belonging to a single virtual machine.
#[derive(Debug, Clone)]
pub struct VmPaths {
    dir: PathBuf,
}

impl VmPaths {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }

    pub fn disk(&self) -> PathBuf {
        self.dir.join(DISK_FILE)
    }

    /// This VM's private copy of the OVMF variable store. Secure Boot keys and
    /// the boot order live here, so it must never be shared between guests.
    pub fn vars(&self) -> PathBuf {
        self.dir.join(VARS_FILE)
    }

    pub fn exists(&self) -> bool {
        self.dir.is_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> VmName {
        VmName::new(text).unwrap()
    }

    #[test]
    fn lays_a_vm_out_under_its_own_directory() {
        let paths = Paths::at("/data/daholyvm");
        let vm = paths.vm(&name("win11"));

        assert_eq!(vm.dir(), Path::new("/data/daholyvm/vms/win11"));
        assert_eq!(
            vm.config(),
            Path::new("/data/daholyvm/vms/win11/config.toml")
        );
        assert_eq!(vm.disk(), Path::new("/data/daholyvm/vms/win11/disk.qcow2"));
        assert_eq!(
            vm.vars(),
            Path::new("/data/daholyvm/vms/win11/OVMF_VARS.fd")
        );
    }

    #[test]
    fn every_vm_gets_its_own_variable_store() {
        let paths = Paths::at("/data/daholyvm");
        assert_ne!(paths.vm(&name("a")).vars(), paths.vm(&name("b")).vars());
    }

    #[test]
    fn a_vm_directory_stays_under_the_vms_directory() {
        // Belt and braces: VmName already rejects traversal, but the layout
        // must not be the only thing standing between a name and `$HOME`.
        let paths = Paths::at("/data/daholyvm");
        for text in ["win11", "a.b", "x-1"] {
            let dir = paths.vm(&name(text)).dir().to_owned();
            assert!(dir.starts_with(paths.vms_dir()), "{dir:?} escaped vms/");
        }
    }

    #[test]
    fn listing_skips_files_and_foreign_directories() {
        let root = std::env::temp_dir().join("daholyvm-paths-list");
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::at(&root);
        std::fs::create_dir_all(paths.vms_dir()).unwrap();
        std::fs::create_dir(paths.vms_dir().join("win11")).unwrap();
        std::fs::create_dir(paths.vms_dir().join("win10")).unwrap();
        std::fs::create_dir(paths.vms_dir().join(".scratch")).unwrap();
        std::fs::write(paths.vms_dir().join("notes.txt"), "").unwrap();

        let found: Vec<String> = paths.list().iter().map(|n| n.to_string()).collect();
        assert_eq!(found, vec!["win10", "win11"]);
    }

    #[test]
    fn listing_an_absent_store_is_empty_not_an_error() {
        assert!(Paths::at("/nonexistent/daholyvm").list().is_empty());
    }
}
