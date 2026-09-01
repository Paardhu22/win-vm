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

pub mod config;
pub mod error;
pub mod paths;
pub mod preflight;

pub use error::{Error, Result};
