# ADR 0001: DA-HOLY-VM orchestrates existing virtualization, it does not implement one

- Status: accepted
- Date: 2026-08-31

## Context

DA-HOLY-VM's goal is to make running Windows on Linux simple. There are two ways
to get there: build virtualization from scratch, or make the existing Linux
stack pleasant to use.

## Decision

DA-HOLY-VM is an orchestration and user-experience layer over QEMU, KVM and
OVMF. It will not implement a hypervisor, a CPU emulator or device models.

## Consequences

- The hard, security-critical parts are handled by mature, audited components.
- Effort goes where the actual pain is: capability detection, sane defaults,
  correct QEMU command lines, clean shutdown, and comprehensible errors.
- DA-HOLY-VM inherits QEMU's and KVM's capabilities and limitations, and depends
  on their presence on the host — which makes detection and remediation
  messaging a first-class feature rather than an afterthought.
