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
use crate::paths::{Paths, VmPaths, MAX_SOCKET_PATH};
use crate::preflight::{HostReport, Package, QEMU_IMG_BINARY, QEMU_SYSTEM_BINARY, SWTPM_BINARY};
use crate::qemu::{args, runtime, Running};
use crate::{disk, tpm, Error, Result};

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

        // swtpm has to be listening before QEMU starts, because QEMU connects
        // to the socket as it comes up and fails outright if it is not there.
        let emulator = if self.config.tpm {
            Some(self.start_tpm(host)?)
        } else {
            None
        };

        let argv = args::build(&self.config, host, &self.paths)?;
        let mut running = runtime::spawn(&qemu_system, &argv)?;

        // From here the emulator's lifetime is the guest's, including on the
        // paths where the guest dies early.
        if let Some(child) = emulator {
            running.adopt(SWTPM_BINARY, child);
        }

        Ok(running)
    }

    /// Start the software TPM this guest is configured with.
    fn start_tpm(&self, host: &HostReport) -> Result<std::process::Child> {
        self.start_tpm_within(host, tpm::SOCKET_TIMEOUT)
    }

    /// The above, with the wait made explicit so tests need not sit through it.
    fn start_tpm_within(
        &self,
        host: &HostReport,
        timeout: std::time::Duration,
    ) -> Result<std::process::Child> {
        let swtpm = host
            .tpm
            .swtpm
            .as_ref()
            .ok_or_else(|| Error::TpmUnavailable {
                remedy: format!(
                    "`{SWTPM_BINARY}` was not found on PATH; {}, or recreate the VM with --no-tpm \
                 (Windows 11 will then refuse to install)",
                    host.platform.package_manager().install_hint(Package::Swtpm)
                ),
            })?;

        let socket = self.paths.tpm_socket();
        // Caught by name here, because the kernel silently truncates instead
        // and QEMU then fails with a path nobody recognises.
        let length = socket.as_os_str().len();
        if length > MAX_SOCKET_PATH {
            return Err(Error::SocketPathTooLong {
                path: socket.to_owned(),
                length,
                limit: MAX_SOCKET_PATH,
            });
        }

        let state = self.paths.tpm_state();
        std::fs::create_dir_all(&state).map_err(|source| Error::write(&state, source))?;
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::write(parent, source))?;
        }

        let child = tpm::start(swtpm, &state, socket)?;
        tpm::wait_for_socket(socket, timeout)?;
        Ok(child)
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
    fn a_tpm_vm_on_a_host_without_swtpm_says_what_to_install() {
        let root = scratch("no-swtpm");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11"));
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();
        let vm = Vm {
            config,
            paths: vm_paths,
        };

        let err = vm.start_tpm(&host_without_qemu()).unwrap_err();
        assert!(matches!(err, Error::TpmUnavailable { .. }), "{err}");
        let text = err.to_string();
        assert!(text.contains("swtpm"), "{text}");
        assert!(
            text.contains("--no-tpm"),
            "the way out must be offered: {text}"
        );
    }

    #[test]
    fn an_overlong_socket_path_is_rejected_by_name() {
        // The kernel truncates instead, and QEMU then fails with a path nobody
        // recognises, so this must be caught here.
        let root = scratch("long-socket");
        let paths = Paths::at(root.join("a".repeat(120)));
        let config = VmConfig::new(name("win11"));
        let vm_paths = paths.vm(&config.name);
        let vm = Vm {
            config,
            paths: vm_paths,
        };

        let host = host_without_qemu_with_tpm(Some(PathBuf::from("/usr/bin/swtpm")));
        let err = vm.start_tpm(&host).unwrap_err();
        assert!(matches!(err, Error::SocketPathTooLong { .. }), "{err}");
    }

    #[test]
    fn a_vm_without_a_tpm_never_looks_for_swtpm() {
        let root = scratch("no-tpm-configured");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win10")).with_tpm(false);
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();
        let vm = Vm {
            config,
            paths: vm_paths,
        };

        // The host has no swtpm at all; launch must fail on QEMU instead.
        let err = vm.launch(&host_without_qemu()).unwrap_err();
        assert!(
            !matches!(err, Error::TpmUnavailable { .. }),
            "a VM with tpm = false must not require swtpm: {err}"
        );
    }

    /// A stand-in for swtpm: creates its socket after a short delay, then
    /// stays alive. The delay is the point — it proves the launch waits.
    fn fake_swtpm(dir: &std::path::Path) -> PathBuf {
        let script = dir.join("fake-swtpm");
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             for a in \"$@\"; do\n\
             case \"$a\" in type=unixio,path=*) sock=${a#type=unixio,path=} ;; esac\n\
             done\n\
             sleep 0.2\n\
             : > \"$sock\"\n\
             sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(
            &script,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
        )
        .unwrap();
        script
    }

    #[test]
    fn the_tpm_is_started_and_waited_for_before_the_guest() {
        let root = scratch("tpm-sequencing");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11"));
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();
        let vm = Vm {
            config,
            paths: vm_paths,
        };

        let host = host_without_qemu_with_tpm(Some(fake_swtpm(&root)));
        let mut child = vm.start_tpm(&host).expect("the emulator must start");

        // start_tpm only returns once the socket exists, because QEMU would
        // otherwise fail to connect to it.
        assert!(
            vm.paths().tpm_socket().exists(),
            "start_tpm returned before the socket appeared"
        );
        // State is persistent and belongs to the guest, not to this run.
        assert!(vm.paths().tpm_state().is_dir());

        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn an_emulator_that_never_listens_times_out_instead_of_hanging() {
        let root = scratch("tpm-timeout");
        let paths = Paths::at(&root);
        let config = VmConfig::new(name("win11"));
        let vm_paths = paths.vm(&config.name);
        std::fs::create_dir_all(vm_paths.dir()).unwrap();

        // `true` stands in for an swtpm that exits without ever listening.
        let host = host_without_qemu_with_tpm(Some(PathBuf::from("/bin/true")));
        let vm = Vm {
            config,
            paths: vm_paths,
        };

        let err = vm
            .start_tpm_within(&host, std::time::Duration::from_millis(80))
            .unwrap_err();
        assert!(matches!(err, Error::HelperTimeout { .. }), "{err}");
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
