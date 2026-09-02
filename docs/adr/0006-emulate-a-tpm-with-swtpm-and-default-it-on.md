# ADR 0006: Emulate a TPM with swtpm, and default it on

- Status: accepted
- Date: 2026-09-02

## Context

Windows 11 setup checks for a TPM 2.0 and stops if it does not find one. The
message it shows — "This PC can't run Windows 11" — names none of the
requirements it checked, so a user meets this as an unexplained dead end
several minutes into an installation.

QEMU emulates no TPM itself. It speaks to an external emulator over a unix
socket, and `swtpm` is that emulator. A VM with a TPM is therefore two
processes, not one, which is a real increase in what the tool has to manage:
startup ordering, a socket path, persistent state, and cleanup.

The alternative is to document the registry bypass that skips the requirements
check during setup. That works, but it means telling the user to defeat a
security check on their own machine as a matter of routine, and it produces an
installation Microsoft considers unsupported.

## Decision

DA-HOLY-VM starts `swtpm` alongside QEMU and connects the two, and `tpm = true`
is the default for a new VM. `--no-tpm` turns it off.

`swtpm` being absent from the host is a **warning**, not a blocker: a VM still
starts without one, and Windows 10 does not care.

## Consequences

- A default `daholyvm create` produces a VM that Windows 11 will install on,
  which is the whole point of the project.
- A VM is now up to two processes. `qemu::runtime` owns the emulator's
  lifetime, including on the paths where the guest dies early, because a stray
  `swtpm` holding a stale socket is exactly what makes the *next* launch fail.
- Startup is ordered: `swtpm` must be listening before QEMU starts, since QEMU
  connects as it comes up and fails outright otherwise. The launch waits for
  the socket to appear, with a timeout, rather than racing.
- TPM state is persistent and lives with the VM. It holds the guest's
  endorsement key and anything Windows seals against it, BitLocker keys
  included — deleting it is equivalent to replacing the machine's motherboard,
  and the guest will treat it that way.
- Sockets go under `$XDG_RUNTIME_DIR`, not beside the VM. Unix socket paths are
  capped at 108 bytes, which a long home directory plus a 64 character VM name
  can exceed; the kernel truncates silently and QEMU then fails with a path
  nobody recognises. The limit is checked by name, and the fallback location is
  used only when `XDG_RUNTIME_DIR` is unset.
- Hosts without `swtpm` are not shut out. They get a `doctor` warning that says
  what to install and what it costs them, and a launch error offering
  `--no-tpm` as the way through.
