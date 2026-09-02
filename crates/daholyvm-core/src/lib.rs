//! Core orchestration logic for DA-HOLY-VM.
//!
//! DA-HOLY-VM is an orchestration and user-experience layer around existing
//! Linux virtualization infrastructure (QEMU + KVM + OVMF). This crate holds
//! all of the domain logic and deliberately contains no GUI or CLI code, so it
//! can be driven equally well by the command line front end, the future desktop
//! GUI, or the test suite.
//!
//! The modules stack from the host upwards:
//!
//! - [`preflight`] — what this machine can do, and what to install if it cannot
//! - [`config`] — what a virtual machine is, and the rules for a valid one
//! - [`paths`] — where virtual machines live on disk
//! - [`disk`] — creating qcow2 images through `qemu-img`
//! - [`qemu`] — building a QEMU command line, and running it
//! - [`tpm`] — the software TPM 2.0 Windows 11 insists on
//! - [`vm`] — the lifecycle that ties those together

pub mod config;
pub mod disk;
pub mod error;
pub mod paths;
pub mod preflight;
pub mod qemu;
pub mod tpm;
pub mod vm;

pub use error::{Error, Result};
pub use vm::Vm;
