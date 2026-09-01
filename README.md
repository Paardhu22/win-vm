# DA-HOLY-VM

Simple Windows virtual machines for Linux.

DA-HOLY-VM is an orchestration and user-experience layer over the Linux
virtualization stack. It does **not** implement a hypervisor or a CPU emulator —
it drives QEMU, KVM and OVMF, and takes responsibility for the parts that are
normally fiddly: knowing whether your machine is capable, building a correct
QEMU command line, and shutting a guest down cleanly.

## Status

Milestones 1 and 2 of the MVP: host capability detection, and a virtual machine
you can create and boot.

```
daholyvm doctor                                  # can this host run a VM?
daholyvm create win11 --iso ~/Win11.iso          # make one
daholyvm run win11                               # boot it
daholyvm list                                    # what exists
```

`doctor` exits `0` when the host can launch a VM and `1` when a required
component is missing. Every failed check carries a distribution-specific
remedy rather than a bare "not found".

`create` makes a directory per VM holding its configuration, its qcow2 disk and
its own UEFI variable store. `run` boots it in the foreground and waits; shut
Windows down from inside the guest to stop it.

### Not yet

- **No TPM.** Windows 11 checks for TPM 2.0 during setup and will refuse to
  install without it. Windows 10 guests are unaffected. Emulating one means
  driving `swtpm`, which is the next thing to build.
- Storage is emulated AHCI rather than virtio, because the Windows installer
  has no virtio driver in the box and would show an empty disk list.
- Stopping a VM from outside it is a power cut. Asking the guest to shut down
  politely needs QEMU's QMP socket.

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

Virtual machines live under `$XDG_DATA_HOME/daholyvm/vms/<name>/`, one
directory each, so a VM can be backed up or deleted as a unit.

Architectural decisions are recorded in [`docs/adr/`](docs/adr/), and the
overall design is described in [`docs/architecture.md`](docs/architecture.md).

## Development

```
cargo test          # core logic is unit tested and needs no QEMU installed
cargo clippy --all-targets
cargo fmt
```
