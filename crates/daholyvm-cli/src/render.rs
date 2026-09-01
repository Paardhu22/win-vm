//! Human-readable rendering of core types.
//!
//! Formatting only. Every decision this prints has already been made by
//! `daholyvm-core`, so the GUI can present the same facts differently
//! without reimplementing any of them.

use daholyvm_core::paths::Paths;
use daholyvm_core::preflight::{format_kib, HostReport, Requirement, Status};
use daholyvm_core::Vm;

const WIDTH: usize = 78;
const REMEDY_INDENT: &str = "        ";

pub fn print_report(report: &HostReport) {
    println!();
    println!("DA-HOLY-VM preflight");
    println!();

    let requirements = report.requirements();
    let label_width = requirements
        .iter()
        .map(|r| r.title.chars().count())
        .max()
        .unwrap_or(0);

    for requirement in &requirements {
        print_requirement(requirement, label_width);
    }

    println!();
    print_resources(report);
    println!();
    print_summary(report, &requirements);
    println!();
}

fn print_requirement(requirement: &Requirement, label_width: usize) {
    println!(
        "  {}  {:<label_width$}  {}",
        marker(requirement.status),
        requirement.title,
        requirement.detail,
        label_width = label_width,
    );
    if let Some(remedy) = &requirement.remedy {
        for line in wrap(remedy, WIDTH - REMEDY_INDENT.len()) {
            println!("{REMEDY_INDENT}{line}");
        }
    }
}

fn marker(status: Status) -> char {
    match status {
        Status::Ok => '+',
        Status::Warn => '!',
        Status::Missing => 'x',
    }
}

fn print_resources(report: &HostReport) {
    let memory = match (
        report.platform.memory_total_kib,
        report.platform.memory_available_kib,
    ) {
        (Some(total), Some(available)) => {
            format!(
                "{} total, {} available",
                format_kib(total),
                format_kib(available)
            )
        }
        (Some(total), None) => format_kib(total),
        _ => "unknown".to_owned(),
    };
    println!(
        "  Host resources: {} logical cores, {memory} RAM",
        report.cpu.logical_cores
    );
}

fn print_summary(report: &HostReport, requirements: &[Requirement]) {
    let blockers = requirements
        .iter()
        .filter(|r| r.status == Status::Missing)
        .count();

    if blockers > 0 {
        let noun = if blockers == 1 { "item" } else { "items" };
        println!("  Not ready: {blockers} required {noun} must be resolved before a VM can start.");
        return;
    }

    if report.accelerated() {
        println!("  Ready: this host can run hardware-accelerated Windows virtual machines.");
    } else {
        println!(
            "  Ready, but without KVM acceleration. QEMU would fall back to software emulation,"
        );
        println!("  which is far too slow for a usable Windows guest. Resolve the warnings above.");
    }
}

pub fn print_created(vm: &Vm) {
    let config = vm.config();
    println!();
    println!("Created `{}`", config.name);
    println!(
        "  {} vCPU, {} MiB RAM, {} GiB disk",
        config.cpus, config.memory_mib, config.disk_gib
    );
    println!("  {}", vm.paths().dir().display());
    println!();

    match &config.iso {
        Some(iso) => {
            println!("  Boots from {}", iso.display());
            println!(
                "  Start the installation with: daholyvm run {}",
                config.name
            );
        }
        None => {
            println!("  No installation medium is set. Add one to config.toml, or recreate");
            println!("  the VM with --iso, then: daholyvm run {}", config.name);
        }
    }
    println!();
}

pub fn print_running(vm: &Vm, pid: u32) {
    let config = vm.config();
    println!();
    println!("Starting `{}` (qemu pid {pid})", config.name);
    println!("  Shut the guest down from inside Windows to stop it cleanly.");
    println!();
}

pub fn print_list(vms: &[Vm], paths: &Paths) {
    if vms.is_empty() {
        println!();
        println!("No virtual machines yet.");
        println!("  Create one with: daholyvm create win11 --iso /path/to/Windows.iso");
        println!();
        return;
    }

    let name_width = vms
        .iter()
        .map(|vm| vm.config().name.as_str().chars().count())
        .max()
        .unwrap_or(0);

    println!();
    for vm in vms {
        let config = vm.config();
        println!(
            "  {:<name_width$}  {} vCPU, {} MiB RAM, {} GiB disk",
            config.name.as_str(),
            config.cpus,
            config.memory_mib,
            config.disk_gib,
            name_width = name_width,
        );
    }
    println!();
    println!("  Stored in {}", paths.vms_dir().display());
    println!();
}

/// Wrap text to `width` columns on whitespace boundaries.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_on_word_boundaries_without_losing_words() {
        let text = "add your account to the kvm group and log back in";
        let lines = wrap(text, 20);
        assert!(lines.iter().all(|l| l.chars().count() <= 20), "{lines:?}");
        assert_eq!(lines.join(" "), text);
    }

    #[test]
    fn a_word_longer_than_the_width_is_not_dropped() {
        let lines = wrap("short supercalifragilisticexpialidocious", 10);
        assert_eq!(lines, vec!["short", "supercalifragilisticexpialidocious"]);
    }

    #[test]
    fn empty_text_wraps_to_no_lines() {
        assert!(wrap("   ", 10).is_empty());
    }
}
