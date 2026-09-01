# ADR 0005: Attach the system disk over emulated AHCI, not virtio

- Status: accepted
- Date: 2026-09-01

## Context

QEMU can present a disk to a guest as a paravirtualized virtio device or as an
emulated SATA/AHCI controller. virtio is substantially faster: it is a purpose
built transport with no device emulation in the path, and it is what every
guide recommends for a Linux guest.

The Windows installer, however, ships no virtio driver. Booting it against a
virtio disk produces a disk selection screen listing no disks — a dead end that
looks like a broken tool rather than a missing driver, arriving after the user
has already waited through the installer's boot.

The usual workaround is to attach a second ISO of virtio-win drivers and have
the user load them from the installer. That is precisely the kind of step
DA-HOLY-VM exists to remove.

## Decision

The system disk and the installation medium are attached to an emulated
`ich9-ahci` controller as `ide-hd` and `ide-cd` devices.

## Consequences

- An unmodified Windows ISO boots, finds the disk and installs, with nothing
  for the user to fetch or load.
- Disk throughput is lower than virtio would give. For an interactive desktop
  guest this is noticeable under sustained I/O and irrelevant the rest of the
  time; correctness of the first-run experience is worth more.
- The choice is not permanent. Once DA-HOLY-VM can hand the guest a virtio-win
  medium and drive an unattended install, the same VM can be switched to virtio
  after installation, since Windows can boot from a device whose driver is
  already present. That is a change to `qemu::args` and a config field, not a
  change to any of the surrounding design.
- `qemu::args` asserts the absence of virtio block devices, so this decision
  cannot be reverted by accident.
