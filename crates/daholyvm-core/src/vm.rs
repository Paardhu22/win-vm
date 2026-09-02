//! A virtual machine's lifecycle: create it, load it, launch it.
//!
//! This is the orchestration the project is named for. Each of the modules
//! below it does one thing and knows nothing about the others — [`crate::config`]
//! models a guest, [`crate::paths`] says where it lives, [`crate::disk`] makes
//! images, [`crate::qemu`] builds and runs a command line — and this module is
//! the only place that knows the order they go in.
//!
//! Both front ends drive these three calls, so the CLI and the future GUI
//! cannot drift apart on what "create a VM" means.

use crate::config::{VmConfig, VmName};
use crate::paths::{Paths, VmPaths};
use crate::preflight::{HostReport, QEMU_IMG_BINARY, QEMU_SYSTEM_BINARY};
use crate::qemu::{args, runtime, Running};
use crate::{disk, Error, Result};

/// A virtual machine that exists on disk.
#[derive(Debug, Clone)]
pub struct Vm {
    config: VmConfig,
    paths: VmPaths,
}

impl Vm {
    /// Create a VM: its directory, its disk and its saved configuration.
    ///
    /// Ordered so that the cheap checks fail first. Nothing is written until
    /// the configuration is known to be valid and the VM is known not to exist,
    /// because a half-created VM is worse than none.
    pub fn create(config: VmConfig, paths: &Paths, host: &HostReport) -> Result<Self> {
        config.validate()?;

        let vm_paths = paths.vm(&config.name);
        if vm_paths.exists() {
            return Err(Error::VmExists(config.name.to_string()));
        }

        let qemu_img = host
            .qemu
            .img
            .as_ref()
            .ok_or(Error::MissingBinary {
                binary: QEMU_IMG_BINARY,
            })?
            .path
            .clone();

        std::fs::create_dir_all(vm_paths.dir())
            .map_err(|source| Error::write(vm_paths.dir(), source))?;

        disk::create(&qemu_img, &vm_paths.disk(), config.disk_gib)?;

        let vm = Vm {
            config,
            paths: vm_paths,
        };
        vm.save()?;
        Ok(vm)
    }

    /// Load a VM that already exists.
    pub fn load(name: &VmName, paths: &Paths) -> Result<Self> {
        let vm_paths = paths.vm(name);
        if !vm_paths.exists() {
            return Err(Error::NoSuchVm(name.to_string()));
        }

        let path = vm_paths.config();
        let text = std::fs::read_to_string(&path).map_err(|source| Error::read(&path, source))?;
        let config = VmConfig::from_toml(&text, &path)?;

        Ok(Vm {
            config,
            paths: vm_paths,
        })
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }

    pub fn paths(&self) -> &VmPaths {
        &self.paths
    }

    /// Write the configuration back out.
    pub fn save(&self) -> Result<()> {
        let path = self.paths.config();
        std::fs::write(&path, self.config.to_toml()?).map_err(|source| Error::write(&path, source))
    }

    /// Boot the guest.
    ///
    /// Refuses on a host preflight says cannot launch: QEMU's own diagnostics
    /// for a missing binary or absent firmware are considerably worse than the
    /// remedies `doctor` already has.
    pub fn launch(&self, host: &HostReport) -> Result<Running> {
        if !host.can_launch() {
            return Err(Error::HostNotReady(
                "run `daholyvm doctor` to see what is missing",
            ));
        }

        if let Some(iso) = &self.config.iso {
            // Checked here rather than in `VmConfig::validate`, which is pure:
            // an ISO can be moved or unmounted long after the VM was created.
            if !iso.is_file() {
                return Err(Error::MissingIso(iso.clone()));
            }
        }

        let qemu_system = host
            .qemu
            .system
            .as_ref()
            .ok_or(Error::MissingBinary {
                binary: QEMU_SYSTEM_BINARY,
            })?
            .path
            .clone();

        self.provision_vars(host)?;

        let argv = args::build(&self.config, host, &self.paths)?;
        runtime::spawn(&qemu_system, &argv)
    }

    /// Give this VM its own copy of the OVMF variable store.
    ///
    /// The distribution's `VARS` file is a template shared by every guest on
    /// the machine, and it lives under `/usr`. Each VM needs a private writable
    /// copy, made once and then left alone — recopying would discard the boot
    /// order and Secure Boot keys the guest has since written.
    fn provision_vars(&self, host: &HostReport) -> Result<()> {
        let vars = self.paths.vars();
        if vars.exists() {
            return Ok(());
        }

        let template = &host
            .firmware
            .best()
            .ok_or(Error::HostNotReady(
                "no OVMF UEFI firmware was found, and Windows will not boot without it",
            ))?
            .vars_template;

        std::fs::copy(template, &vars).map_err(|source| Error::write(&vars, source))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preflight::{Cpu, Firmware, FirmwarePair, Kvm, Platform, Qemu, Tpm, VirtExtension};
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("daholyvm-vm-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn host_without_qemu() -> HostReport {
        host_without_qemu_with_tpm(None)
    }

    fn host_without_qemu_with_tpm(swtpm: Option<PathBuf>) -> HostReport {
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
                logical_cores: 4,
                virt: Some(VirtExtension::IntelVtx),
            },
            kvm: Kvm::Ready,
            qemu: Qemu {
                system: None,
                img: None,
            },
            firmware: Firmware {
                found: vec![FirmwarePair {
                    code: PathBuf::from("/code.fd"),
                    vars_template: PathBuf::from("/vars.fd"),
                    secure_boot: true,
                    flash_size_mb: 4,
                    origin: "test",
                }],
            },
            tpm: Tpm { swtpm },
        }
    }

    fn name(text: &str) -> VmName {
        VmName::new(text).unwrap()
    }

    #[test]
    fn creating_without_qemu_img_reports_the_missing_binary() {
        let paths = Paths::at(scratch("no-qemu-img"));
        let config = VmConfig::new(name("win11"));
        let err = Vm::create(config, &paths, &host_without_qemu()).unwrap_err();
        assert!(matches!(err, Error::MissingBinary { .. }), "{err}");
    }

    #[test]
    fn an_invalid_config_is_rejected_before_anything_is_written() {
        let root = scratch("invalid-config");
        let paths = Paths::at(&root);
        let mut config = VmConfig::new(name("win11"));
        config.cpus = 0;

        assert!(Vm::create(config, &paths, &host_without_qemu()).is_err());
        assert!(
            !paths.vms_dir().exists(),
            "nothing may be created for a config that was never valid"
        );
    }

    #[test]
    fn loading_an_absent_vm_names_it() {
        let paths = Paths::at(scratch("absent"));
        let err = Vm::load(&name("ghost"), &paths).unwrap_err();
        assert!(matches!(err, Error::NoSuchVm(_)), "{err}");
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn a_saved_vm_loads_back_unchanged() {
        let root = scratch("round-trip");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11")).with_iso(Some(PathBuf::from("/iso/Win11.iso")));

        // Stand in for `create`, which would need a real qemu-img.
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();
        let vm = Vm {
            config: config.clone(),
            paths: vm_paths,
        };
        vm.save().unwrap();

        let loaded = Vm::load(&config.name, &paths).unwrap();
        assert_eq!(loaded.config(), &config);
    }

    #[test]
    fn creating_a_vm_that_already_exists_is_refused() {
        let root = scratch("duplicate");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11"));
        std::fs::create_dir_all(paths.vm(&config.name).dir()).unwrap();

        let err = Vm::create(config, &paths, &host_without_qemu()).unwrap_err();
        assert!(matches!(err, Error::VmExists(_)), "{err}");
    }

    #[test]
    fn launching_on_an_unready_host_points_at_doctor() {
        let root = scratch("unready");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11"));
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();
        let vm = Vm {
            config,
            paths: vm_paths,
        };

        // The host has no QEMU, so preflight reports a blocker.
        let err = vm.launch(&host_without_qemu()).unwrap_err();
        assert!(err.to_string().contains("doctor"), "{err}");
    }

    #[test]
    fn an_existing_variable_store_is_never_overwritten() {
        let root = scratch("vars");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11"));
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();
        std::fs::write(vm_paths.vars(), b"the guest's own boot order").unwrap();

        let vm = Vm {
            config,
            paths: vm_paths,
        };
        vm.provision_vars(&host_without_qemu()).unwrap();

        assert_eq!(
            std::fs::read(vm.paths().vars()).unwrap(),
            b"the guest's own boot order",
            "recopying the template would discard the guest's Secure Boot state"
        );
    }
}
