//! The software TPM, run as a second process alongside QEMU.
//!
//! Windows 11 will not install without a TPM 2.0, and QEMU does not emulate one
//! itself — it connects over a unix socket to an external emulator. `swtpm` is
//! that emulator, so a VM with a TPM is two processes, started in order:
//!
//! ```text
//! swtpm socket --tpmstate dir=<vm>/tpm --ctrl type=unixio,path=<socket> --tpm2
//!     |
//!     +-- unix socket --> qemu-system-x86_64 -chardev socket,... -device tpm-tis
//! ```
//!
//! The state directory holds the guest's own TPM: its endorsement key, and
//! anything Windows seals against it including BitLocker keys. It lives with
//! the VM and is never treated as a cache.
//!
//! As with [`crate::qemu::args`], the argument vector is a pure function so the
//! exact invocation is testable without swtpm installed.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use crate::preflight::SWTPM_BINARY;
use crate::{Error, Result};

/// How long to wait for swtpm to create its socket before giving up. Starting
/// the emulator is local and fast; a wait this long means it is not coming.
pub const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

/// `swtpm socket --tpmstate dir=… --ctrl type=unixio,path=… --tpm2`
pub fn socket_args(state_dir: &Path, socket: &Path) -> Vec<OsString> {
    let mut dir = OsString::from("dir=");
    dir.push(state_dir);

    let mut ctrl = OsString::from("type=unixio,path=");
    ctrl.push(socket);

    vec![
        OsString::from("socket"),
        OsString::from("--tpmstate"),
        dir,
        OsString::from("--ctrl"),
        ctrl,
        // TPM 2.0 specifically. swtpm still defaults to the 1.2 spec, which
        // Windows 11 does not accept.
        OsString::from("--tpm2"),
    ]
}

/// Start the emulator. It runs until killed; [`crate::qemu::runtime`] owns it
/// from here and stops it when the guest exits.
pub fn start(swtpm: &Path, state_dir: &Path, socket: &Path) -> Result<Child> {
    Command::new(swtpm)
        .args(socket_args(state_dir, socket))
        .spawn()
        .map_err(|source| Error::Spawn {
            binary: SWTPM_BINARY.to_owned(),
            source,
        })
}

/// Block until the control socket appears.
///
/// QEMU connects to the socket as it starts and fails outright if it is not
/// there yet, so the two processes cannot simply be launched together.
pub fn wait_for_socket(socket: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if socket.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    Err(Error::HelperTimeout {
        binary: SWTPM_BINARY,
        path: socket.to_owned(),
        seconds: timeout.as_secs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn builds_a_tpm_2_socket_invocation() {
        let args = socket_args(
            Path::new("/data/vms/win11/tpm"),
            Path::new("/run/user/1000/daholyvm/win11-swtpm.sock"),
        );
        assert_eq!(
            args,
            vec![
                OsString::from("socket"),
                OsString::from("--tpmstate"),
                OsString::from("dir=/data/vms/win11/tpm"),
                OsString::from("--ctrl"),
                OsString::from("type=unixio,path=/run/user/1000/daholyvm/win11-swtpm.sock"),
                OsString::from("--tpm2"),
            ]
        );
    }

    #[test]
    fn always_asks_for_tpm_2_because_windows_11_rejects_1_2() {
        let args = socket_args(Path::new("/state"), Path::new("/sock"));
        assert!(args.contains(&OsString::from("--tpm2")));
    }

    #[test]
    fn a_missing_swtpm_is_reported_as_a_spawn_failure() {
        let err = start(
            Path::new("/nonexistent/swtpm"),
            Path::new("/state"),
            Path::new("/sock"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }), "{err}");
        assert!(err.to_string().contains("swtpm"));
    }

    #[test]
    fn waiting_returns_as_soon_as_the_socket_exists() {
        let dir = std::env::temp_dir().join("daholyvm-tpm-present");
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("swtpm.sock");
        std::fs::write(&socket, b"").unwrap();

        assert!(wait_for_socket(&socket, Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn waiting_gives_up_and_names_the_socket_it_wanted() {
        let socket = PathBuf::from("/nonexistent/never-appears.sock");
        let err = wait_for_socket(&socket, Duration::from_millis(60)).unwrap_err();
        assert!(matches!(err, Error::HelperTimeout { .. }), "{err}");
        assert!(err.to_string().contains("never-appears.sock"));
    }
}
