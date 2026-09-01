//! Command line front end for DA-HOLY-VM.
//!
//! This binary is intentionally thin: it parses arguments, calls into
//! `daholyvm-core`, and renders the result. All domain logic lives in the core
//! crate so that the future desktop GUI can reuse it unchanged.

mod render;

use clap::{Parser, Subcommand};
use daholyvm_core::preflight::HostReport;

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
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Doctor { json } => doctor(json),
    }
}

fn doctor(json: bool) -> std::process::ExitCode {
    let report = HostReport::detect();

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(err) => {
                eprintln!("daholyvm: failed to serialize report: {err}");
                return std::process::ExitCode::FAILURE;
            }
        }
    } else {
        render::print_report(&report);
    }

    if report.can_launch() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}
