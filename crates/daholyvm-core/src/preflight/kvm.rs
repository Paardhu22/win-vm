//! KVM availability.
//!
//! KVM is an *acceleration* dependency, not a hard one: QEMU will happily fall
//! back to TCG emulation. That fallback is roughly an order of magnitude slower
//! and makes a Windows guest unusable in practice, so its absence is reported
//! loudly — but it does not block launching.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;

use serde::Serialize;

use super::{Cpu, Requirement, Status, Sysroot};

/// Canonical location of the KVM character device.
pub const KVM_DEVICE: &str = "/dev/kvm";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Kvm {
    /// The device exists and this process can open it read/write.
    Ready,
    /// The device exists but this user cannot open it, almost always a group
    /// membership problem.
    PermissionDenied {
        /// Octal permission bits, e.g. `660`.
        mode: String,
        /// Owning group name, resolved from `/etc/group` when possible.
        group: Option<String>,
    },
    /// No `/dev/kvm` at all: the `kvm` kernel modules are not loaded, or the
    /// CPU has no virtualization extensions to drive them.
    DeviceMissing,
    /// The device exists but failed to open for some other reason.
    Unavailable { message: String },
}

impl Kvm {
    pub fn detect_in(root: &Sysroot) -> Self {
        let path = root.resolve(KVM_DEVICE);
        let Ok(metadata) = std::fs::metadata(&path) else {
            return Kvm::DeviceMissing;
        };

        // Opening the device read/write is exactly what QEMU does, and is the
        // only reliable permission test: mode bits alone miss ACLs.
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(_) => Kvm::Ready,
            Err(err) if err.kind() == ErrorKind::PermissionDenied => Kvm::PermissionDenied {
                mode: format!("{:o}", metadata.mode() & 0o777),
                group: group_name(root, metadata.gid()),
            },
            Err(err) => Kvm::Unavailable {
                message: err.to_string(),
            },
        }
    }

    pub(crate) fn requirement(&self, cpu: &Cpu) -> Requirement {
        const TITLE: &str = "KVM acceleration";
        const ID: &str = "kvm.device";

        match self {
            Kvm::Ready => Requirement::new(
                ID,
                TITLE,
                Status::Ok,
                format!("{KVM_DEVICE} is present and writable"),
            ),
            Kvm::PermissionDenied { mode, group } => {
                let group = group.as_deref().unwrap_or("kvm");
                Requirement::new(
                    ID,
                    TITLE,
                    Status::Warn,
                    format!("{KVM_DEVICE} exists (mode {mode}, group `{group}`) but is not writable by this user"),
                )
                .with_remedy(format!(
                    "add your account to the `{group}` group and log back in: \
                     sudo usermod -aG {group} $USER"
                ))
            }
            Kvm::DeviceMissing if cpu.virt.is_none() => Requirement::new(
                ID,
                TITLE,
                Status::Warn,
                format!("{KVM_DEVICE} is absent because the CPU reports no virtualization extensions"),
            )
            .with_remedy("resolve the CPU virtualization check above first"),
            Kvm::DeviceMissing => Requirement::new(
                ID,
                TITLE,
                Status::Warn,
                format!("{KVM_DEVICE} is absent although the CPU supports virtualization"),
            )
            .with_remedy(
                "load the KVM kernel module (`sudo modprobe kvm_intel` or `sudo modprobe kvm_amd`); \
                 if that fails, virtualization is most likely disabled in your BIOS/UEFI setup",
            ),
            Kvm::Unavailable { message } => {
                Requirement::new(ID, TITLE, Status::Warn, format!("{KVM_DEVICE}: {message}"))
                    .with_remedy("check that no other hypervisor (VirtualBox, VMware) holds the device")
            }
        }
    }
}

/// Resolve a numeric gid to a group name via `/etc/group`.
fn group_name(root: &Sysroot, gid: u32) -> Option<String> {
    parse_group_name(&root.read("/etc/group")?, gid)
}

/// Look up a gid in `/etc/group` content. Format: `name:passwd:gid:members`.
pub fn parse_group_name(text: &str, gid: u32) -> Option<String> {
    for line in text.lines() {
        let mut fields = line.split(':');
        let name = fields.next()?;
        let _passwd = fields.next();
        let Some(Ok(entry_gid)) = fields.next().map(str::parse::<u32>) else {
            continue;
        };
        if entry_gid == gid {
            return Some(name.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUP: &str = "root:x:0:\nkvm:x:960:\nwheel:x:998:paardhu\n";

    #[test]
    fn resolves_gid_to_group_name() {
        assert_eq!(parse_group_name(GROUP, 960).as_deref(), Some("kvm"));
        assert_eq!(parse_group_name(GROUP, 0).as_deref(), Some("root"));
    }

    #[test]
    fn unknown_gid_is_none() {
        assert_eq!(parse_group_name(GROUP, 4242), None);
    }

    #[test]
    fn malformed_group_lines_are_skipped() {
        assert_eq!(
            parse_group_name("garbage\nkvm:x:960:\n", 960).as_deref(),
            Some("kvm")
        );
    }

    #[test]
    fn absent_device_is_reported_as_missing_not_denied() {
        let root = Sysroot::at(std::env::temp_dir().join("daholyvm-nonexistent-root"));
        assert_eq!(Kvm::detect_in(&root), Kvm::DeviceMissing);
    }

    #[test]
    fn permission_denied_remedy_names_the_owning_group() {
        let cpu = Cpu {
            model: None,
            logical_cores: 1,
            virt: Some(super::super::VirtExtension::IntelVtx),
        };
        let kvm = Kvm::PermissionDenied {
            mode: "660".into(),
            group: Some("kvm".into()),
        };
        let remedy = kvm.requirement(&cpu).remedy.unwrap();
        assert!(remedy.contains("usermod -aG kvm"), "remedy was: {remedy}");
    }
}
