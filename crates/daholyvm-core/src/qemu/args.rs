//! Build the QEMU command line.
//!
//! This is the pure function ADR 0003 asks for: `(VmConfig, HostReport,
//! VmPaths) -> Vec<OsString>`. Nothing here touches the filesystem, spawns a
//! process or prints, so the entire command line a user would get can be
//! asserted in a unit test on a machine with no QEMU installed. That is the
//! single highest-value test surface in the project, because a wrong flag here
//! surfaces as a guest that will not boot, hours later.
//!
//! The device choices are deliberate and are the difference between a Windows
//! installer that runs and one that stops on a black screen or an empty disk
//! list. Each is explained where it is made.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use crate::config::VmConfig;
use crate::paths::VmPaths;
use crate::preflight::{FirmwarePair, HostReport};
use crate::{Error, Result};

/// Build the argument vector for `qemu-system-x86_64`.
///
/// Fails only when the host has no UEFI firmware, since a Windows guest cannot
/// be expressed as a command line without one.
pub fn build(config: &VmConfig, host: &HostReport, paths: &VmPaths) -> Result<Vec<OsString>> {
    let firmware = host.firmware.best().ok_or(Error::HostNotReady(
        "no OVMF UEFI firmware was found, and Windows will not boot without it",
    ))?;

    let mut argv = Argv::default();

    argv.flag("-name", config.name.as_str());

    machine(&mut argv, firmware);
    processor(&mut argv, config, host);
    argv.flag("-m", config.memory_mib.to_string());

    firmware_flash(&mut argv, firmware, paths);
    storage(&mut argv, config, paths);
    if config.tpm {
        tpm(&mut argv, paths);
    }
    peripherals(&mut argv);

    Ok(argv.into_vec())
}

/// q35 is the modern chipset; OVMF and current Windows both expect it, and the
/// older i440fx has no PCIe and no SMM.
fn machine(argv: &mut Argv, firmware: &FirmwarePair) {
    // SMM is what keeps the guest from writing its own Secure Boot variables.
    // Enabling it without a Secure Boot firmware only costs startup time.
    let smm = if firmware.secure_boot { "on" } else { "off" };
    argv.flag("-machine", format!("q35,smm={smm}"));

    // Windows guests under QEMU hang rather than resume from S3, and the guest
    // offers "Sleep" in its own menus, so the state is genuinely reachable.
    argv.flag("-global", "ICH9-LPC.disable_s3=1");

    // Windows keeps the hardware clock in local time and will otherwise drift
    // by the timezone offset on every boot.
    argv.flag("-rtc", "base=localtime");
}

/// Hardware acceleration when the host has it, and a plain emulated CPU when it
/// does not. `-cpu host` is meaningless under TCG, where nothing is passed
/// through.
fn processor(argv: &mut Argv, config: &VmConfig, host: &HostReport) {
    if host.accelerated() {
        argv.flag("-accel", "kvm");
        argv.flag("-cpu", "host");
    } else {
        argv.flag("-accel", "tcg");
        argv.flag("-cpu", "qemu64");
    }
    argv.flag("-smp", config.cpus.to_string());
}

/// OVMF is attached as two pflash units: the read-only firmware itself, and
/// this VM's private, writable variable store.
fn firmware_flash(argv: &mut Argv, firmware: &FirmwarePair, paths: &VmPaths) {
    if firmware.secure_boot {
        // Without this the variable store is writable from the guest and
        // Secure Boot is decorative.
        argv.flag("-global", "driver=cfi.pflash01,property=secure,value=on");
    }
    argv.flag_os(
        "-drive",
        escaped(
            "if=pflash,format=raw,unit=0,readonly=on,file=",
            &firmware.code,
        ),
    );
    argv.flag_os(
        "-drive",
        escaped("if=pflash,format=raw,unit=1,file=", &paths.vars()),
    );
}

/// The system disk and, while installing, the medium.
///
/// Both hang off an emulated AHCI controller rather than virtio. virtio is
/// considerably faster, but the Windows installer has no virtio driver in the
/// box: it would boot to a disk selection screen listing no disks. Switching to
/// virtio once the guest can be given drivers is a later milestone.
fn storage(argv: &mut Argv, config: &VmConfig, paths: &VmPaths) {
    argv.flag("-device", "ich9-ahci,id=sata");

    argv.flag_os(
        "-drive",
        escaped("id=hd,if=none,format=qcow2,file=", &paths.disk()),
    );
    argv.flag("-device", "ide-hd,drive=hd,bus=sata.0");

    match &config.iso {
        Some(iso) => {
            argv.flag_os(
                "-drive",
                escaped(
                    "id=cd,if=none,format=raw,media=cdrom,readonly=on,file=",
                    iso,
                ),
            );
            argv.flag("-device", "ide-cd,drive=cd,bus=sata.1");
            // Try the medium first, then the disk, so the same command line
            // installs Windows and then boots what it installed.
            argv.flag("-boot", "order=dc");
        }
        None => argv.flag("-boot", "order=c"),
    }
}

/// Connect the guest to the swtpm process started alongside it.
///
/// QEMU emulates no TPM of its own; `tpm-tis` is the interface the guest sees,
/// `tpmdev emulator` is the backend, and the chardev is the socket the emulator
/// is listening on. Windows 11 setup checks for this and stops without it.
fn tpm(argv: &mut Argv, paths: &VmPaths) {
    argv.flag_os(
        "-chardev",
        escaped("socket,id=chrtpm,path=", paths.tpm_socket()),
    );
    argv.flag("-tpmdev", "emulator,id=tpm0,chardev=chrtpm");
    // tpm-tis is the interface Windows expects on a q35 machine.
    argv.flag("-device", "tpm-tis,tpmdev=tpm0");
}

fn peripherals(argv: &mut Argv) {
    // e1000e is emulated rather than fast, but Windows has the driver in the
    // box, so the guest has working networking during installation.
    argv.flag("-netdev", "user,id=net0");
    argv.flag("-device", "e1000e,netdev=net0");

    // A USB tablet reports absolute coordinates. Without it the host and guest
    // pointers drift apart and the window has to grab the mouse.
    argv.flag("-device", "qemu-xhci,id=usb");
    argv.flag("-device", "usb-tablet,bus=usb.0");

    argv.flag("-vga", "std");
}

/// Build a QEMU option-list value ending in a path, escaping the path.
///
/// QEMU splits these values on commas, so a comma inside a path has to be
/// doubled or the rest of the filename is read as further options. Paths are
/// handled as bytes because a filename need not be UTF-8.
fn escaped(options: &str, path: &Path) -> OsString {
    let mut bytes = Vec::from(options.as_bytes());
    for &byte in path.as_os_str().as_bytes() {
        if byte == b',' {
            bytes.push(b',');
        }
        bytes.push(byte);
    }
    OsString::from_vec(bytes)
}

/// A growing argument vector. Exists so the builder above reads as a list of
/// decisions rather than a wall of `push`.
#[derive(Default)]
struct Argv(Vec<OsString>);

impl Argv {
    fn flag(&mut self, name: &str, value: impl Into<String>) {
        self.flag_os(name, OsString::from(value.into()));
    }

    fn flag_os(&mut self, name: &str, value: OsString) {
        self.0.push(OsString::from(name));
        self.0.push(value);
    }

    fn into_vec(self) -> Vec<OsString> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::VmName;
    use crate::paths::Paths;
    use crate::preflight::{Cpu, Firmware, Kvm, Platform, Qemu, Tpm, VirtExtension};
    use std::path::PathBuf;

    fn firmware_pair(secure_boot: bool) -> FirmwarePair {
        FirmwarePair {
            code: PathBuf::from("/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd"),
            vars_template: PathBuf::from("/usr/share/edk2/x64/OVMF_VARS.4m.fd"),
            secure_boot,
            flash_size_mb: 4,
            origin: "test",
        }
    }

    fn host(kvm: Kvm, firmware: Vec<FirmwarePair>) -> HostReport {
        HostReport {
            platform: Platform {
                is_linux: true,
                arch: "x86_64",
                kernel: None,
                os_release: Default::default(),
                memory_total_kib: None,
                memory_available_kib: None,
            },
            cpu: Cpu {
                model: None,
                logical_cores: 8,
                virt: Some(VirtExtension::IntelVtx),
            },
            kvm,
            qemu: Qemu {
                system: None,
                img: None,
            },
            firmware: Firmware { found: firmware },
            tpm: Tpm {
                swtpm: Some(PathBuf::from("/usr/bin/swtpm")),
            },
        }
    }

    fn ready_host() -> HostReport {
        host(Kvm::Ready, vec![firmware_pair(true)])
    }

    fn config(iso: Option<&str>) -> VmConfig {
        VmConfig::new(VmName::new("win11").unwrap()).with_iso(iso.map(PathBuf::from))
    }

    fn vm_paths() -> VmPaths {
        Paths::at("/data/daholyvm").vm(&VmName::new("win11").unwrap())
    }

    fn build_ok(config: &VmConfig, host: &HostReport) -> Vec<String> {
        build(config, host, &vm_paths())
            .unwrap()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// The value that follows `flag`, when it appears exactly once.
    fn value_of(argv: &[String], flag: &str) -> Option<String> {
        let at = argv.iter().position(|a| a == flag)?;
        argv.get(at + 1).cloned()
    }

    fn values_of(argv: &[String], flag: &str) -> Vec<String> {
        argv.windows(2)
            .filter(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
            .collect()
    }

    #[test]
    fn uses_kvm_and_the_host_cpu_when_the_host_is_accelerated() {
        let argv = build_ok(&config(None), &ready_host());
        assert_eq!(value_of(&argv, "-accel").as_deref(), Some("kvm"));
        assert_eq!(value_of(&argv, "-cpu").as_deref(), Some("host"));
    }

    #[test]
    fn falls_back_to_emulation_without_kvm() {
        let host = host(Kvm::DeviceMissing, vec![firmware_pair(true)]);
        let argv = build_ok(&config(None), &host);
        assert_eq!(value_of(&argv, "-accel").as_deref(), Some("tcg"));
        // `-cpu host` is meaningless under TCG and QEMU refuses it.
        assert_eq!(value_of(&argv, "-cpu").as_deref(), Some("qemu64"));
    }

    #[test]
    fn passes_the_configured_sizing_through() {
        let mut config = config(None);
        config.cpus = 6;
        config.memory_mib = 8192;
        let argv = build_ok(&config, &ready_host());
        assert_eq!(value_of(&argv, "-smp").as_deref(), Some("6"));
        assert_eq!(value_of(&argv, "-m").as_deref(), Some("8192"));
    }

    #[test]
    fn secure_boot_firmware_enables_smm_and_locks_the_variable_store() {
        let argv = build_ok(&config(None), &ready_host());
        assert_eq!(value_of(&argv, "-machine").as_deref(), Some("q35,smm=on"));
        assert!(values_of(&argv, "-global")
            .iter()
            .any(|v| v == "driver=cfi.pflash01,property=secure,value=on"));
    }

    #[test]
    fn plain_firmware_leaves_smm_off_and_does_not_claim_to_be_secure() {
        let host = host(Kvm::Ready, vec![firmware_pair(false)]);
        let argv = build_ok(&config(None), &host);
        assert_eq!(value_of(&argv, "-machine").as_deref(), Some("q35,smm=off"));
        assert!(!values_of(&argv, "-global")
            .iter()
            .any(|v| v.contains("property=secure")));
    }

    #[test]
    fn attaches_the_firmware_read_only_and_the_vm_s_own_variable_store_writable() {
        let argv = build_ok(&config(None), &ready_host());
        let drives = values_of(&argv, "-drive");

        let code = drives.iter().find(|d| d.contains("OVMF_CODE")).unwrap();
        assert!(
            code.contains("readonly=on"),
            "firmware must not be writable"
        );

        let vars = drives.iter().find(|d| d.contains("OVMF_VARS")).unwrap();
        assert!(
            vars.contains("/data/daholyvm/vms/win11/OVMF_VARS.fd"),
            "the VM's private copy must be used, not the template: {vars}"
        );
        assert!(!vars.contains("readonly"));
    }

    #[test]
    fn puts_the_system_disk_on_ahci_because_windows_has_no_virtio_driver() {
        let argv = build_ok(&config(None), &ready_host());
        assert!(values_of(&argv, "-device")
            .iter()
            .any(|d| d == "ich9-ahci,id=sata"));
        assert!(values_of(&argv, "-device")
            .iter()
            .any(|d| d.starts_with("ide-hd")));
        assert!(
            !argv
                .iter()
                .any(|a| a.contains("virtio-blk") || a.contains("virtio-scsi")),
            "the installer would show no disks: {argv:?}"
        );
        assert!(values_of(&argv, "-drive")
            .iter()
            .any(|d| d.contains("/data/daholyvm/vms/win11/disk.qcow2")));
    }

    #[test]
    fn an_iso_is_attached_as_a_cdrom_and_booted_first() {
        let argv = build_ok(&config(Some("/iso/Win11.iso")), &ready_host());
        let cd = values_of(&argv, "-drive")
            .into_iter()
            .find(|d| d.contains("media=cdrom"))
            .expect("a cdrom drive");
        assert!(cd.contains("/iso/Win11.iso"));
        assert!(cd.contains("readonly=on"));
        assert_eq!(value_of(&argv, "-boot").as_deref(), Some("order=dc"));
    }

    #[test]
    fn without_an_iso_the_disk_is_the_only_boot_device() {
        let argv = build_ok(&config(None), &ready_host());
        assert_eq!(value_of(&argv, "-boot").as_deref(), Some("order=c"));
        assert!(!argv.iter().any(|a| a.contains("media=cdrom")));
    }

    #[test]
    fn a_comma_in_a_path_is_escaped_rather_than_read_as_further_options() {
        // `-drive` values are comma separated, so an unescaped comma would turn
        // the rest of the filename into options QEMU then rejects.
        let argv = build_ok(&config(Some("/iso/Windows 11, 24H2.iso")), &ready_host());
        let cd = values_of(&argv, "-drive")
            .into_iter()
            .find(|d| d.contains("media=cdrom"))
            .unwrap();
        assert!(
            cd.ends_with("file=/iso/Windows 11,, 24H2.iso"),
            "comma must be doubled: {cd}"
        );
    }

    #[test]
    fn every_flag_has_a_value_and_paths_stay_single_arguments() {
        let argv = build_ok(&config(Some("/iso/a b;c.iso")), &ready_host());
        assert_eq!(argv.len() % 2, 0, "flags and values must pair up: {argv:?}");
        assert!(argv
            .chunks(2)
            .all(|pair| pair[0].starts_with('-') && !pair[1].starts_with('-')));
        assert!(argv.iter().any(|a| a.contains("/iso/a b;c.iso")));
    }

    #[test]
    fn a_tpm_is_wired_to_the_swtpm_socket_by_default() {
        let argv = build_ok(&config(None), &ready_host());
        assert_eq!(
            value_of(&argv, "-chardev").as_deref(),
            Some("socket,id=chrtpm,path=/data/daholyvm/vms/win11/tpm/swtpm.sock")
        );
        assert_eq!(
            value_of(&argv, "-tpmdev").as_deref(),
            Some("emulator,id=tpm0,chardev=chrtpm")
        );
        assert!(values_of(&argv, "-device")
            .iter()
            .any(|d| d == "tpm-tis,tpmdev=tpm0"));
    }

    #[test]
    fn turning_the_tpm_off_removes_every_trace_of_it() {
        let argv = build_ok(&config(None).with_tpm(false), &ready_host());
        assert!(
            !argv.iter().any(|a| a.contains("tpm")),
            "no TPM flags may survive: {argv:?}"
        );
    }

    #[test]
    fn a_host_without_firmware_cannot_produce_a_command_line() {
        let host = host(Kvm::Ready, Vec::new());
        let err = build(&config(None), &host, &vm_paths()).unwrap_err();
        assert!(matches!(err, Error::HostNotReady(_)), "{err}");
    }
}
