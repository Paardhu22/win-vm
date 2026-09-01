//! Host capability detection ("preflight").
//!
//! Everything here is read-only: preflight never modifies the host. Its job is
//! to answer "can this machine run a Windows guest, and if not, what exactly
//! should the user do about it?". Each probe therefore carries remediation
//! text alongside its boolean result — a bare `false` is not a useful error.

mod cpu;
mod distro;
mod firmware;
mod kvm;
mod platform;
mod qemu;
mod sysroot;
mod which;

pub use cpu::{Cpu, VirtExtension};
pub use distro::{OsRelease, Package, PackageManager};
pub use firmware::{Firmware, FirmwarePair};
pub use kvm::Kvm;
pub use platform::{format_kib, parse_meminfo, Platform};
pub use qemu::{Qemu, QemuBinary, Version, MIN_QEMU_VERSION, QEMU_IMG_BINARY, QEMU_SYSTEM_BINARY};
pub use sysroot::Sysroot;

use serde::Serialize;

/// Outcome of a single preflight check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Requirement satisfied.
    Ok,
    /// Usable, but degraded or suspicious. Does not block launching a VM.
    Warn,
    /// Hard blocker. A VM cannot be launched until this is resolved.
    Missing,
}

/// A single human-facing line of the preflight report.
#[derive(Debug, Clone, Serialize)]
pub struct Requirement {
    /// Stable machine-readable identifier, safe for the GUI to match on.
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    /// What was actually found.
    pub detail: String,
    /// What the user should do about it, when there is something to do.
    pub remedy: Option<String>,
}

impl Requirement {
    fn new(
        id: &'static str,
        title: &'static str,
        status: Status,
        detail: impl Into<String>,
    ) -> Self {
        Requirement {
            id,
            title,
            status,
            detail: detail.into(),
            remedy: None,
        }
    }

    fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }
}

/// The full set of preflight findings for a host.
#[derive(Debug, Clone, Serialize)]
pub struct HostReport {
    pub platform: Platform,
    pub cpu: Cpu,
    pub kvm: Kvm,
    pub qemu: Qemu,
    pub firmware: Firmware,
}

impl HostReport {
    /// Probe the running host.
    pub fn detect() -> Self {
        Self::detect_in(&Sysroot::host())
    }

    /// Probe a filesystem tree. Production code passes [`Sysroot::host`]; the
    /// test suite passes a fixture directory so detection can be exercised on
    /// machines that have no QEMU or firmware installed.
    pub fn detect_in(root: &Sysroot) -> Self {
        let platform = Platform::detect_in(root);
        HostReport {
            cpu: Cpu::detect_in(root),
            kvm: Kvm::detect_in(root),
            qemu: Qemu::detect(),
            firmware: Firmware::detect_in(root),
            platform,
        }
    }

    /// The report rendered as an ordered checklist.
    pub fn requirements(&self) -> Vec<Requirement> {
        let pm = self.platform.package_manager();
        vec![
            self.platform.requirement(),
            self.cpu.requirement(),
            self.kvm.requirement(&self.cpu),
            self.qemu.system_requirement(pm),
            self.qemu.img_requirement(pm),
            self.firmware.requirement(pm),
        ]
    }

    /// True when nothing is outright missing, i.e. a VM can be launched.
    pub fn can_launch(&self) -> bool {
        self.requirements()
            .iter()
            .all(|r| r.status != Status::Missing)
    }

    /// True when the guest can use hardware acceleration rather than falling
    /// back to pure emulation (which is far too slow for a Windows guest).
    pub fn accelerated(&self) -> bool {
        matches!(self.kvm, Kvm::Ready)
    }
}
