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
//!     tpm/            swtpm's state: the guest's TPM keys
//! ```
//!
//! The one thing that does **not** live there is the swtpm control socket.
//! Unix socket paths are limited to 108 bytes, which a long home directory and
//! a 64 character VM name can exceed, so sockets go under `$XDG_RUNTIME_DIR`
//! — which is both short and the correct place for them — falling back to the
//! VM directory when that is unset.
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
pub const TPM_DIR: &str = "tpm";

const APP_DIR: &str = "daholyvm";
const VMS_DIR: &str = "vms";

/// `sun_path` is 108 bytes including its terminator on Linux. Exceeding it
/// fails inside QEMU with a truncated path and a baffling message, so it is
/// worth catching by name.
pub const MAX_SOCKET_PATH: usize = 107;

/// The root of DA-HOLY-VM's storage.
#[derive(Debug, Clone)]
pub struct Paths {
    data: PathBuf,
    /// Where transient sockets go. `None` falls back to the VM's directory.
    runtime: Option<PathBuf>,
}

impl Paths {
    /// Resolve from the environment, honouring `XDG_DATA_HOME`.
    pub fn from_env() -> Result<Self> {
        let data = match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            // The XDG spec's own fallback when the variable is unset or empty.
            _ => PathBuf::from(std::env::var_os("HOME").ok_or(Error::NoHome)?).join(".local/share"),
        };
        let runtime = match std::env::var_os("XDG_RUNTIME_DIR") {
            Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir).join(APP_DIR)),
            _ => None,
        };

        Ok(Paths {
            data: data.join(APP_DIR),
            runtime,
        })
    }

    /// Storage rooted at an explicit directory, used by the tests.
    pub fn at(data: impl Into<PathBuf>) -> Self {
        Paths {
            data: data.into(),
            runtime: None,
        }
    }

    /// Storage with an explicit runtime directory, used by the tests.
    pub fn at_with_runtime(data: impl Into<PathBuf>, runtime: impl Into<PathBuf>) -> Self {
        Paths {
            data: data.into(),
            runtime: Some(runtime.into()),
        }
    }

    /// Where sockets for this VM go, if anywhere better than its own directory.
    pub fn runtime_dir(&self) -> Option<&Path> {
        self.runtime.as_deref()
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
        let dir = self.vms_dir().join(name.as_str());
        let tpm_socket = match &self.runtime {
            Some(runtime) => runtime.join(format!("{}-swtpm.sock", name.as_str())),
            None => dir.join(TPM_DIR).join("swtpm.sock"),
        };
        VmPaths { dir, tpm_socket }
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
    tpm_socket: PathBuf,
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

    /// swtpm's persistent state: the guest's own TPM keys live here, so this
    /// is kept with the VM and never in a temporary directory.
    pub fn tpm_state(&self) -> PathBuf {
        self.dir.join(TPM_DIR)
    }

    /// The swtpm control socket QEMU connects to.
    pub fn tpm_socket(&self) -> &Path {
        &self.tpm_socket
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
    fn tpm_state_stays_with_the_vm_but_the_socket_goes_to_the_runtime_dir() {
        let paths = Paths::at_with_runtime("/data/daholyvm", "/run/user/1000/daholyvm");
        let vm = paths.vm(&name("win11"));

        assert_eq!(vm.tpm_state(), Path::new("/data/daholyvm/vms/win11/tpm"));
        assert_eq!(
            vm.tpm_socket(),
            Path::new("/run/user/1000/daholyvm/win11-swtpm.sock")
        );
    }

    #[test]
    fn without_a_runtime_dir_the_socket_falls_back_beside_the_vm() {
        let vm = Paths::at("/data/daholyvm").vm(&name("win11"));
        assert_eq!(
            vm.tpm_socket(),
            Path::new("/data/daholyvm/vms/win11/tpm/swtpm.sock")
        );
    }

    #[test]
    fn a_runtime_socket_path_stays_well_inside_the_unix_limit() {
        // The whole reason sockets are not kept beside the VM: this must hold
        // even for the longest name the validator accepts.
        let paths = Paths::at_with_runtime(
            "/home/somebody/.local/share/daholyvm",
            "/run/user/1000/daholyvm",
        );
        let longest = VmName::new("a".repeat(crate::config::MAX_NAME_LEN)).unwrap();
        let socket = paths.vm(&longest).tpm_socket().as_os_str().len();
        assert!(
            socket <= MAX_SOCKET_PATH,
            "{socket} bytes exceeds the limit"
        );
    }

    #[test]
    fn listing_an_absent_store_is_empty_not_an_error() {
        assert!(Paths::at("/nonexistent/daholyvm").list().is_empty());
    }
}
