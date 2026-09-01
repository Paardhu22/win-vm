//! The QEMU child process: start it, wait for it, stop it.
//!
//! Deliberately thin. The guest's own shutdown is the normal way a VM ends —
//! the user shuts Windows down from inside it and QEMU exits on its own — so
//! the job here is to start the process correctly and report honestly on how it
//! finished.
//!
//! `kill` is a hard stop, equivalent to pulling the power cord, and is offered
//! for a guest that has stopped responding. Asking the guest to shut down
//! politely means driving QEMU's QMP socket, which is the next milestone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

use crate::preflight::QEMU_SYSTEM_BINARY;
use crate::{Error, Result};

/// A running guest.
#[derive(Debug)]
pub struct Running {
    child: Child,
    binary: PathBuf,
}

/// Start `qemu-system-x86_64` with an already-built argument vector.
pub fn spawn(qemu_system: &Path, argv: &[OsString]) -> Result<Running> {
    // Inherited stdio: QEMU's own diagnostics belong in the user's terminal,
    // where they can see why a guest refused to start.
    let child = Command::new(qemu_system)
        .args(argv)
        .spawn()
        .map_err(|source| Error::Spawn {
            binary: QEMU_SYSTEM_BINARY.to_owned(),
            source,
        })?;

    Ok(Running {
        child,
        binary: qemu_system.to_owned(),
    })
}

impl Running {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Block until the guest exits.
    ///
    /// A non-zero exit is returned rather than raised: QEMU exiting because the
    /// user closed the window is not an error, and the caller is better placed
    /// to decide what a given status means.
    pub fn wait(&mut self) -> Result<ExitStatus> {
        self.child.wait().map_err(|source| Error::Spawn {
            binary: self.binary.display().to_string(),
            source,
        })
    }

    /// Stop the guest immediately, without telling it first.
    ///
    /// This is a power cut: an unclean shutdown a Windows guest will complain
    /// about on its next boot. Use it when the guest is already wedged.
    pub fn kill(&mut self) -> Result<()> {
        self.child.kill().map_err(|source| Error::Spawn {
            binary: self.binary.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_binary_is_reported_as_a_spawn_failure() {
        let err = spawn(Path::new("/nonexistent/qemu-system-x86_64"), &[]).unwrap_err();
        assert!(matches!(err, Error::Spawn { .. }), "{err}");
        assert!(err.to_string().contains("qemu-system-x86_64"));
    }

    #[test]
    fn waits_for_a_child_and_reports_its_status() {
        // `true` stands in for QEMU: the point is that the wrapper reports a
        // real exit status rather than swallowing it.
        let mut running = spawn(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("exit 3")],
        )
        .expect("sh must be present");
        assert!(running.pid() > 0);
        let status = running.wait().unwrap();
        assert_eq!(status.code(), Some(3));
    }

    #[test]
    fn kill_stops_a_running_child() {
        let mut running = spawn(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("sleep 30")],
        )
        .expect("sh must be present");
        running.kill().unwrap();
        let status = running.wait().unwrap();
        assert!(!status.success(), "a killed child must not report success");
    }
}
