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
                |  vm         the lifecycle   |
                |  qemu::args build argv      |
                |  qemu::runtime  the process |
                |  tpm        swtpm           |
                |  disk       qemu-img        |
                |  paths      where VMs live  |
                |  config     model a VM      |
                |  preflight  detect the host |
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

### config, paths, disk

`VmConfig` is the whole of what DA-HOLY-VM knows about a guest, persisted as
TOML beside its disk so it stays readable and hand-editable. Validation is
**pure** — it inspects values, never the filesystem or the host — which is what
makes the whole rule set unit testable. Whether the host can satisfy a config is
preflight's question; whether an ISO still exists is checked at launch.

`VmName` is a validated newtype rather than a sanitiser. Rewriting `../../etc`
into something harmless would mean the VM the user asked for and the VM they got
have different names, so the name is rejected instead.

Storage is one directory per VM under `$XDG_DATA_HOME/daholyvm/vms/<name>/`,
holding `config.toml`, `disk.qcow2` and `OVMF_VARS.fd`. Grouping by VM rather
than by file type means a guest can be backed up, copied or deleted as a unit.
Each VM gets a private copy of the OVMF variable store, made once: it holds the
boot order and Secure Boot keys the guest writes, so recopying the distribution
template would silently discard them.

### qemu

`args::build` is `(VmConfig, HostReport, VmPaths) -> Vec<OsString>` and is pure,
so the exact command line a user would get is asserted in unit tests on a
machine with no QEMU installed. A wrong flag here surfaces as a guest that will
not boot, hours later, which is why this is the most heavily tested surface in
the project.

The device choices are guest-driven and are recorded where they are made:

| Choice | Why |
| --- | --- |
| `q35` machine | OVMF and modern Windows both expect it; i440fx has no PCIe or SMM |
| AHCI, not virtio | the Windows installer ships no virtio driver and would list no disks |
| `e1000e` NIC | Windows has the driver in the box, so networking works during setup |
| `-rtc base=localtime` | Windows keeps the hardware clock in local time |
| `disable_s3=1` | Windows guests hang rather than resume from S3 |
| `smm=on` + `pflash01 secure` | otherwise the guest can write its own Secure Boot variables |
| `tpm-tis` + `tpmdev emulator` | Windows 11 setup stops without a TPM 2.0; QEMU emulates none itself |

`runtime` owns the child process, and any helper processes serving it. Stopping
a VM is currently a hard kill; a graceful ACPI shutdown means driving QEMU's QMP
socket.

### tpm

A guest with a TPM is two processes. QEMU emulates no TPM itself — it speaks to
`swtpm` over a unix socket — so `vm::launch` starts the emulator, waits for its
socket to appear rather than racing it, and hands the child to `runtime` so that
whichever way QEMU ends, nothing is left running. A stray `swtpm` holding a
stale socket is exactly what makes the *next* launch fail.

TPM state is persistent and lives with the VM: it holds the guest's endorsement
key and anything Windows seals against it. The socket does not, because unix
socket paths are capped at 108 bytes and a long home plus a long VM name can
exceed that; sockets go under `$XDG_RUNTIME_DIR`, and the limit is checked by
name rather than left to the kernel to truncate silently. ADR 0006 has the
reasoning.

### vm

The only module that knows what order the others go in. `Vm::create`,
`Vm::load` and `Vm::launch` are what both front ends call, so the CLI and the
future GUI cannot drift apart on what "create a VM" means.

### Still to build

| Module | Responsibility |
| --- | --- |
| `qemu::monitor` | QMP socket: graceful shutdown, status, running VMs |
| `daholyvm-gui` | desktop front end over the same core types |

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
- `qemu::args` is a pure function, so the entire command line can be asserted in
  unit tests without ever launching a VM;
- `VmConfig` validation is pure, and config storage goes through an injectable
  root, so VM creation and loading are tested inside a temporary directory;
- helper processes are tested against stand-in binaries — `/bin/sh` for QEMU, a
  script that creates a socket after a delay for `swtpm` — so process ordering
  and cleanup are asserted without either being installed.
