# ADR 0003: Build argument vectors, never shell strings

- Status: accepted
- Date: 2026-08-31

## Context

A QEMU command line embeds user-controlled data: ISO paths, disk paths, VM
names, sizes. Assembling that into a string and handing it to a shell would make
every one of those values a potential injection point.

## Decision

QEMU is always invoked through `std::process::Command` with arguments pushed
individually as `OsString`. No shell is ever involved, and no user-provided
string is ever executed as a command.

The command line is produced by a **pure function**,
`(VmConfig, HostReport) -> Vec<OsString>`, separate from the code that spawns
the process.

## Consequences

- A path containing spaces, quotes, `;` or `$()` is inert: it is one argument.
- Paths are handled as `OsString`/`PathBuf` throughout, so non-UTF-8 filenames
  work rather than being lossily mangled.
- The entire command line is unit-testable without QEMU installed, which is the
  single highest-value test surface in the project.
- The only executables DA-HOLY-VM ever runs are `qemu-system-x86_64` and
  `qemu-img`, resolved from `PATH` and version-checked before use.
