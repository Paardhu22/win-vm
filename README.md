# DA-HOLY-VM

Simple Windows virtual machines for Linux.

DA-HOLY-VM is an orchestration and user-experience layer over the Linux
virtualization stack. It does **not** implement a hypervisor or a CPU emulator —
it drives QEMU, KVM and OVMF, and takes responsibility for the parts that are
normally fiddly: knowing whether your machine is capable, building a correct
QEMU command line, and shutting a guest down cleanly.

## Status

Milestone 1 of the MVP: host capability detection.

```
cargo run -p daholyvm-cli -- doctor
cargo run -p daholyvm-cli -- doctor --json
```

`doctor` exits `0` when the host can launch a VM and `1` when a required
component is missing. Every failed check carries a distribution-specific
remedy rather than a bare "not found".

## Requirements

- Linux on x86_64, with Intel VT-x or AMD-V enabled in firmware
- QEMU 6.0 or newer (`qemu-system-x86_64`, `qemu-img`)
- OVMF/edk2 UEFI firmware
- KVM (`/dev/kvm`) for hardware acceleration

Run `daholyvm doctor` — it will tell you which of these you are missing and the
exact command to install it on your distribution.

## Layout

| Crate | Purpose |
| --- | --- |
| `daholyvm-core` | All domain logic. No GUI, no CLI dependencies. |
| `daholyvm-cli` | Thin `daholyvm` binary over the core crate. |

Architectural decisions are recorded in [`docs/adr/`](docs/adr/), and the
overall design is described in [`docs/architecture.md`](docs/architecture.md).

## Development

```
cargo test          # core logic is unit tested and needs no QEMU installed
cargo clippy --all-targets
cargo fmt
```
