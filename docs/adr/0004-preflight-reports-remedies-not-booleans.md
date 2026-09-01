# ADR 0004: Preflight checks report remedies, not booleans

- Status: accepted
- Date: 2026-08-31

## Context

"QEMU not found" is where most Linux virtualization attempts stall. The
information a user needs is not *that* something is missing but *what to type*,
and that differs per distribution — the OVMF package is `edk2-ovmf` on Arch and
Fedora, `ovmf` on Debian, and `qemu-ovmf-x86_64` on openSUSE.

## Decision

Each preflight check returns a `Requirement { status, detail, remedy }`.
`detail` states what was actually found; `remedy` gives the exact command or
action, phrased for the distribution identified from `/etc/os-release`.

Firmware locations are a preference-ordered candidate table, so adding a
distribution means adding rows, not writing code.

## Consequences

- Cross-distribution support is data, and is reviewable as data.
- Both front ends render the same remedies; the GUI will not re-derive them.
- Two failure modes worth calling out are handled explicitly rather than
  collapsing into "not found":
  - `/dev/kvm` exists but is not writable — a group membership problem, so the
    remedy names the owning group resolved from `/etc/group`.
  - A binary is found but is far too old, which in practice means an unrelated
    SDK is shadowing the system install on `PATH`. The remedy shows the resolved
    path and suggests `which -a`.
- Guessing the distribution wrong is harmless: unknown systems fall back to
  naming the upstream component instead of a package.
