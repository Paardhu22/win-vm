//! Command line front end for DA-HOLY-VM.
//!
//! This binary is intentionally thin: it parses arguments, calls into
//! `daholyvm-core`, and renders the result. All domain logic lives in the core
//! crate so that the future desktop GUI can reuse it unchanged.

mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use daholyvm_core::config::{VmConfig, VmName, DEFAULT_CPUS, DEFAULT_DISK_GIB, DEFAULT_MEMORY_MIB};
use daholyvm_core::paths::Paths;
use daholyvm_core::preflight::HostReport;
use daholyvm_core::{Result, Vm};

#[derive(Parser)]
#[command(
    name = "daholyvm",
    about = "DA-HOLY-VM - simple Windows virtual machines for Linux",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check whether this host can run Windows virtual machines.
    Doctor {
        /// Emit the full report as JSON instead of a human-readable checklist.
        #[arg(long)]
        json: bool,
    },

    /// Create a new virtual machine.
    Create {
        /// Name of the virtual machine. Becomes its directory name.
        name: String,

        /// Installation medium to boot from, e.g. a Windows ISO.
        #[arg(long, value_name = "PATH")]
        iso: Option<PathBuf>,

        /// Virtual CPUs.
        #[arg(long, default_value_t = DEFAULT_CPUS)]
        cpus: u32,

        /// Memory in MiB.
        #[arg(long, value_name = "MIB", default_value_t = DEFAULT_MEMORY_MIB)]
        memory: u64,

        /// Disk size in GiB.
        #[arg(long, value_name = "GIB", default_value_t = DEFAULT_DISK_GIB)]
        disk: u64,
    },

    /// Boot a virtual machine and wait for it to shut down.
    Run {
        /// Name of the virtual machine.
        name: String,
    },

    /// List the virtual machines on this system.
    List,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Doctor { json } => doctor(json),
        Command::Create {
            name,
            iso,
            cpus,
            memory,
            disk,
        } => report(create(name, iso, cpus, memory, disk)),
        Command::Run { name } => run(&name),
        Command::List => report(list()),
    }
}

/// Turn a core error into one line on stderr and a failing exit code.
fn report(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("daholyvm: {err}");
            ExitCode::FAILURE
        }
    }
}

fn doctor(json: bool) -> ExitCode {
    let report = HostReport::detect();

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(err) => {
                eprintln!("daholyvm: failed to serialize report: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        render::print_report(&report);
    }

    if report.can_launch() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn create(
    name: String,
    iso: Option<PathBuf>,
    cpus: u32,
    memory_mib: u64,
    disk_gib: u64,
) -> Result<()> {
    let config = VmConfig {
        name: VmName::new(name)?,
        cpus,
        memory_mib,
        disk_gib,
        iso,
    };

    let vm = Vm::create(config, &Paths::from_env()?, &HostReport::detect())?;
    render::print_created(&vm);
    Ok(())
}

/// Boot a VM in the foreground and wait for the guest to shut itself down.
///
/// Waiting rather than detaching is deliberate for now: QEMU's window is the
/// VM, and closing it is how the user ends the session. Background VMs need a
/// control socket to be manageable, which is a later milestone.
fn run(name: &str) -> ExitCode {
    let launched = (|| -> Result<_> {
        let name = VmName::new(name)?;
        let host = HostReport::detect();
        let vm = Vm::load(&name, &Paths::from_env()?)?;
        let running = vm.launch(&host)?;
        Ok((vm, running))
    })();

    let (vm, mut running) = match launched {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("daholyvm: {err}");
            return ExitCode::FAILURE;
        }
    };

    render::print_running(&vm, running.pid());

    match running.wait() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("daholyvm: qemu exited with {status}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("daholyvm: {err}");
            ExitCode::FAILURE
        }
    }
}

fn list() -> Result<()> {
    let paths = Paths::from_env()?;
    let vms: Vec<Vm> = paths
        .list()
        .iter()
        .filter_map(|name| Vm::load(name, &paths).ok())
        .collect();
    render::print_list(&vms, &paths);
    Ok(())
}
