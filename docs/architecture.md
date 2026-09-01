# DA-HOLY-VM architecture

## Principle

DA-HOLY-VM is an **orchestration layer**. Virtualization is performed by KVM,
device emulation by QEMU, and firmware by OVMF. DA-HOLY-VM owns the user
experience around them: capability detection, configuration, disk management,
process lifecycle and error reporting.

No part of this project implements a hypervisor, a CPU emulator or a device
model, and none is planned.

## Layers

```
                +-----------------------------+
                |  daholyvm-gui   (milestone 5)|
                +--------------+--------------+
                               |
                +--------------v--------------+
                |  daholyvm-cli               |
                +--------------+--------------+
                               |
                +--------------v--------------+
                |  daholyvm-core              |
                |                             |
                |  preflight  detect the host |
                |  config     model a VM      |
                |  disk       qemu-img        |
                |  qemu::args build argv      |
                |  qemu::runtime  lifecycle   |
                +--------------+--------------+
                               |
                +--------------v--------------+
                |  QEMU / KVM / OVMF          |
                +-----------------------------+
```

`daholyvm-core` never prints, never reads stdin and has no GUI or CLI
dependencies. Both front ends are renderers over the same types, so anything
the CLI can do the GUI can do without duplicating logic.

### preflight

Read-only probes producing a `HostReport`. Two rules shape this module:

1. **A boolean is not an error message.** Every check yields a `Requirement`
   carrying `detail` (what was actually found) and, where applicable, `remedy`
   (the exact command to fix it, phrased for the detected distribution).
2. **Everything is reachable through a `Sysroot`.** Detection addresses files by
   canonical absolute path but resolves them through a root that tests can
   redirect at a fixture tree. This is what allows preflight to be unit tested
   on a machine with no QEMU or firmware installed.

`Status::Missing` means "a VM cannot start". `Status::Warn` means "it can start,
but you will not like the result" — an absent `/dev/kvm` is a warning, because
QEMU really will fall back to TCG emulation, just far too slowly to be usable.

### Planned modules

| Module | Milestone | Responsibility |
| --- | --- | --- |
| `config` | 2 | `VmConfig` as serde/TOML, plus pure validation |
| `disk` | 2 | Create and inspect qcow2 images via `qemu-img` |
| `qemu::args` | 3 | Pure `(VmConfig, HostReport) -> Vec<OsString>` |
| `qemu::runtime` | 3 | Spawn, monitor and cleanly stop the QEMU child |
| `paths` | 2 | XDG-compliant VM storage locations |

## Safety posture

- QEMU is invoked with an **argument vector**, never a shell string. User input
  (ISO paths, VM names, sizes) becomes a single `OsString` argument and can
  never be reinterpreted as syntax.
- No user-provided string is ever executed as a command. The only executables
  DA-HOLY-VM runs are `qemu-system-x86_64` and `qemu-img`, resolved from `PATH`
  and version-checked.
- VM names are sanitised before being used as filesystem paths.
- Discovered binaries are reported by resolved path, because `PATH` shadowing by
  unrelated SDKs is a real and confusing failure mode.

## Testing

The core crate is testable without QEMU, KVM or firmware present:

- parsers (`/proc/cpuinfo`, `/proc/meminfo`, `/etc/os-release`, `/etc/group`,
  `--version` output) are pure functions over `&str`;
- filesystem probes go through `Sysroot` and run against fixture trees;
- the future `qemu::args` builder is a pure function, so the entire command line
  can be asserted in unit tests without ever launching a VM.
