//! Driving `qemu-system-x86_64`.
//!
//! Split so that deciding *what* to run is separate from *running* it: `args`
//! is a pure function with no side effects and exhaustive tests, `runtime` owns
//! the child process. ADR 0003 explains why the boundary is worth having.

pub mod args;
pub mod runtime;

pub use args::build;
pub use runtime::{spawn, Running};
