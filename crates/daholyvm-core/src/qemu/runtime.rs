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
//!
//! A guest can need more than one process: a VM with a TPM has `swtpm` running
//! beside it. Those helpers are owned here too, so that whichever way QEMU
//! ends — clean shutdown, crash, or kill — nothing is left running.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

use crate::preflight::QEMU_SYSTEM_BINARY;
use crate::{Error, Result};

/// A helper process that exists only to serve one guest.
#[derive(Debug)]
struct Helper {
    name: &'static str,
    child: Child,
}

/// A running guest, and any helper processes it depends on.
#[derive(Debug)]
pub struct Running {
    child: Child,
    binary: PathBuf,
    helpers: Vec<Helper>,
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
        helpers: Vec::new(),
    })
}

impl Running {
    /// Tie a helper's lifetime to the guest's. It is stopped when the guest
    /// exits, however that happens.
    pub fn adopt(&mut self, name: &'static str, child: Child) {
        self.helpers.push(Helper { name, child });
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Names of the helper processes running alongside the guest.
    pub fn helpers(&self) -> Vec<&'static str> {
        self.helpers.iter().map(|helper| helper.name).collect()
    }

    /// Block until the guest exits.
    ///
    /// A non-zero exit is returned rather than raised: QEMU exiting because the
    /// user closed the window is not an error, and the caller is better placed
    /// to decide what a given status means.
    pub fn wait(&mut self) -> Result<ExitStatus> {
        let status = self.child.wait().map_err(|source| Error::Spawn {
            binary: self.binary.display().to_string(),
            source,
        });
        // Helpers exist only to serve this guest, and a stray swtpm holding a
        // stale socket is exactly what makes the *next* launch fail.
        self.stop_helpers();
        status
    }

    /// Stop the guest immediately, without telling it first.
    ///
    /// This is a power cut: an unclean shutdown a Windows guest will complain
    /// about on its next boot. Use it when the guest is already wedged.
    pub fn kill(&mut self) -> Result<()> {
        let killed = self.child.kill().map_err(|source| Error::Spawn {
            binary: self.binary.display().to_string(),
            source,
        });
        self.stop_helpers();
        killed
    }

    /// Best effort, and deliberately infallible: a helper that has already
    /// exited is the outcome we wanted anyway.
    fn stop_helpers(&mut self) {
        for helper in &mut self.helpers {
            let _ = helper.child.kill();
            let _ = helper.child.wait();
        }
        self.helpers.clear();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // Covers the paths that never reach `wait`, such as an early return
        // between spawning the helper and spawning QEMU.
        self.stop_helpers();
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

    fn sleeper() -> Child {
        Command::new("/bin/sh")
            .args(["-c", "sleep 30"])
            .spawn()
            .expect("sh must be present")
    }

    #[test]
    fn helpers_are_stopped_when_the_guest_exits() {
        let mut running = spawn(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("exit 0")],
        )
        .unwrap();

        let helper = sleeper();
        let helper_pid = helper.id();
        running.adopt("swtpm", helper);
        assert_eq!(running.helpers(), vec!["swtpm"]);

        running.wait().unwrap();

        // A stale swtpm holding the socket is what breaks the next launch.
        assert!(
            !process_alive(helper_pid),
            "helper {helper_pid} outlived the guest"
        );
        assert!(running.helpers().is_empty());
    }

    #[test]
    fn helpers_are_stopped_when_the_guest_is_killed() {
        let mut running = spawn(
            Path::new("/bin/sh"),
            &[OsString::from("-c"), OsString::from("sleep 30")],
        )
        .unwrap();

        let helper = sleeper();
        let helper_pid = helper.id();
        running.adopt("swtpm", helper);

        running.kill().unwrap();
        let _ = running.wait();

        assert!(!process_alive(helper_pid));
    }

    #[test]
    fn dropping_a_guest_takes_its_helpers_with_it() {
        let helper_pid = {
            let mut running = spawn(
                Path::new("/bin/sh"),
                &[OsString::from("-c"), OsString::from("exit 0")],
            )
            .unwrap();
            let helper = sleeper();
            let pid = helper.id();
            running.adopt("swtpm", helper);
            pid
        };
        assert!(!process_alive(helper_pid), "drop must not leak a helper");
    }

    /// True while the process exists and has not been reaped.
    fn process_alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
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
