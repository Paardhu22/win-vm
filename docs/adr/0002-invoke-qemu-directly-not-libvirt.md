# ADR 0002: Drive QEMU directly rather than through libvirt

- Status: accepted
- Date: 2026-08-31

## Context

The two ways to launch a QEMU guest are to spawn `qemu-system-x86_64` directly,
or to define a domain in libvirt and let `libvirtd` manage it.

libvirt brings domain XML, a stable API and existing tooling, but also a system
daemon, a privileged socket, polkit rules and per-distribution service
configuration — a substantial share of the setup friction DA-HOLY-VM exists to
remove. Its absence on this development host (no `virsh`, no `libvirtd`) is
representative of a default desktop install.

## Decision

DA-HOLY-VM spawns and owns the QEMU process itself, and controls the running
guest over a QMP unix socket.

## Consequences

- No daemon, no system service, no polkit prompt. Everything runs as the user,
  which user-mode networking and a writable `/dev/kvm` are sufficient for.
- DA-HOLY-VM owns process lifecycle, which is exactly what the "stop the VM
  cleanly" requirement needs: QMP `system_powerdown` asks Windows to shut down
  in an orderly fashion, with SIGTERM only as a timeout fallback.
- We take on responsibility for command line correctness across QEMU versions,
  which is mitigated by a minimum supported version and by the argument builder
  being a pure, heavily tested function.
- Interoperability with `virt-manager` is lost. Acceptable: DA-HOLY-VM's users
  are, by definition, people not already using it.
