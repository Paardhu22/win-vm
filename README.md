# DA-HOLY-VM

Simple Windows virtual machines for Linux.

DA-HOLY-VM is an orchestration and user-experience layer over the Linux
virtualization stack. It does **not** implement a hypervisor or a CPU emulator —
it drives QEMU, KVM and OVMF, and takes responsibility for the parts that are
normally fiddly: knowing whether your machine is capable, building a correct
QEMU command line, and shutting a guest down cleanly.

The whole project exists because the gap between "I have a Linux laptop" and "I
have a working Windows VM" is a dozen decisions that have exactly one right
answer each, and getting any of them wrong produces a black screen with no
explanation.

---

## Table of contents

- [Where the project is](#where-the-project-is)
- [Quick start](#quick-start)
- [How it works](#how-it-works)
  - [The layers](#the-layers)
  - [Step 1 — `daholyvm doctor`](#step-1--daholyvm-doctor)
  - [Step 2 — `daholyvm create`](#step-2--daholyvm-create)
  - [Step 3 — `daholyvm run`](#step-3--daholyvm-run)
  - [The generated QEMU command line](#the-generated-qemu-command-line)
- [On-disk layout](#on-disk-layout)
- [Command reference](#command-reference)
- [Configuration reference](#configuration-reference)
- [Design decisions](#design-decisions)
- [Safety posture](#safety-posture)
- [Testing](#testing)
- [What does not work yet](#what-does-not-work-yet)
- [Roadmap](#roadmap)
- [Troubleshooting](#troubleshooting)
- [Repository layout](#repository-layout)

---

## Where the project is

| Milestone | Scope | State |
| --- | --- | --- |
| 1 | Host capability detection (`doctor`) | **Done** |
| 2 | Config, storage layout, disk creation | **Done** |
| 3 | QEMU command line, process lifecycle (`create`, `run`, `list`) | **Done** |
| 4 | TPM 2.0 via `swtpm`, QMP graceful shutdown | Not started |
| 5 | Desktop GUI | Not started |

Milestones 2 and 3 were built together as a single vertical slice, on the
grounds that you learn what a `VmConfig` actually needs by booting a guest, not
by designing one on paper.

**What works today:** you can check a host, create a VM, and boot it from an
ISO. 80 tests cover it, none of which need QEMU installed.

**The honest caveat:** no Windows guest has been booted end to end yet, because
the development host has neither `qemu-system-x86_64` nor OVMF installed. The
generated command line is asserted by 12 unit tests, and every failure path is
exercised, but "the tests pass" is not "Windows installed". See
[What does not work yet](#what-does-not-work-yet).

---

## Quick start

### 1. Install the host requirements

DA-HOLY-VM needs QEMU and OVMF. It will not install them for you — that needs
root, and a tool that silently runs `sudo` is a tool you cannot trust.

| Distribution | Command |
| --- | --- |
| Arch | `sudo pacman -S --needed qemu-desktop edk2-ovmf` |
| Debian / Ubuntu | `sudo apt install qemu-system-x86 ovmf` |
| Fedora / RHEL | `sudo dnf install qemu-system-x86 edk2-ovmf` |
| openSUSE | `sudo zypper install qemu-x86 qemu-ovmf-x86_64` |

You also want to be in the `kvm` group, or `/dev/kvm` will not open:

```
sudo usermod -aG kvm "$USER"     # then log out and back in
```

If you skip this section, `doctor` will tell you exactly which of these lines to
run, chosen for the distribution you are actually on.

### 2. Build

```
cargo build --release
./target/release/daholyvm doctor
```

### 3. Check the host

```
daholyvm doctor
```

### 4. Create and boot a VM

```
daholyvm create win11 --iso ~/ISOs/Win11.iso
daholyvm run win11
```

`run` opens a QEMU window and blocks until the guest shuts down.

---

## How it works

### The layers

```
                +-----------------------------+
                |  daholyvm-gui   (milestone 5)|
                +--------------+--------------+
                               |
                +--------------v--------------+
                |  daholyvm-cli               |
                |  parse args, render output  |
                +--------------+--------------+
                               |
                +--------------v--------------+
                |  daholyvm-core              |
                |                             |
                |  vm         the lifecycle   |
                |  qemu::args build argv      |
                |  qemu::runtime  the process |
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

`daholyvm-core` never prints, never reads stdin, and has no CLI or GUI
dependencies. Every decision is made there; the CLI only formats the result.
That is what lets the future GUI show the same facts differently without
reimplementing any of them — and it is why `vm.rs` is the *only* module that
knows what order the others go in.

Each module below `vm` does one job and knows nothing about its siblings:

| Module | Lines | Tests | Job |
| --- | --- | --- | --- |
| `preflight` | 1440 | 34 | What can this host do, and what should the user install |
| `config` | 290 | 11 | What a VM is, and the rules for a valid one |
| `paths` | 181 | 5 | Where VMs live on disk |
| `disk` | 141 | 5 | Create qcow2 images through `qemu-img` |
| `qemu::args` | 391 | 12 | Turn a config into a QEMU command line (pure) |
| `qemu::runtime` | 109 | 3 | Own the QEMU child process |
| `vm` | 301 | 7 | The lifecycle that ties those together |

---

### Step 1 — `daholyvm doctor`

Preflight is read-only. It never modifies the host; it answers "can this machine
run a Windows guest, and if not, what exactly should the user do about it?"

Six checks run, in this order:

| # | Check | What it reads | Blocking? |
| --- | --- | --- | --- |
| 1 | Linux host | `/proc/sys/kernel/osrelease`, `/etc/os-release`, `/proc/meminfo` | yes |
| 2 | CPU virtualization extensions | `/proc/cpuinfo` flags (`vmx` / `svm`) | yes |
| 3 | KVM acceleration | `/dev/kvm` existence, mode, owning group | **no** |
| 4 | QEMU system emulator | `qemu-system-x86_64` on `PATH`, `--version` | yes |
| 5 | QEMU disk image tool | `qemu-img` on `PATH`, `--version` | yes |
| 6 | UEFI firmware (OVMF) | 10 known `CODE`/`VARS` locations | yes |

Each produces a `Requirement` with three statuses:

- **`Ok`** — satisfied.
- **`Warn`** — a VM can start, but you will not like the result. An absent
  `/dev/kvm` is only a warning, because QEMU really will fall back to TCG
  software emulation — just far too slowly to be usable for Windows.
- **`Missing`** — a hard blocker. A VM cannot start.

`can_launch()` is "nothing is `Missing`". `doctor` exits `0` when that holds and
`1` when it does not, so it is usable from a script.

**The rule that shapes this whole module: a boolean is not an error message.**
Every check carries `detail` (what was actually found) and, where applicable,
`remedy` (the exact command to fix it, phrased for the detected distribution).
Real output from the development host:

```
DA-HOLY-VM preflight

  +  Linux host                     Arch Linux, kernel 7.1.8-arch1-3, x86_64
  +  CPU virtualization extensions  Intel VT-x present (13th Gen Intel(R) Core(TM) i9-13900H, 20 logical cores)
  +  KVM acceleration               /dev/kvm is present and writable
  x  QEMU system emulator           `qemu-system-x86_64` was not found on PATH
        sudo pacman -S --needed qemu-desktop
  !  QEMU disk image tool           2.12.0 at /home/paardhu/Android/Sdk/emulator/qemu-img is older than the required 6.0.0
        another `qemu-img` earlier in PATH may be shadowing your system
        install (SDKs and toolchains often bundle their own); check `which -a
        qemu-img`, then install or prefer a current QEMU: sudo pacman -S
        --needed qemu-desktop
  x  UEFI firmware (OVMF)           no OVMF firmware pair found in any known location
        sudo pacman -S --needed edk2-ovmf

  Host resources: 20 logical cores, 15.2 GiB total, 7.6 GiB available RAM

  Not ready: 2 required items must be resolved before a VM can start.
```

That `qemu-img` line is the reason binaries are reported by **resolved path**
rather than by name: the Android SDK ships its own ancient `qemu-img` and
shadows the system one on `PATH`. "not found" would have been a lie, and
"found" would have been useless.

`--json` emits the same report as structured data, for the GUI and for scripts.

#### How this is testable without QEMU installed

Two techniques, and they are the reason the test suite runs anywhere:

1. **Parsers are pure functions over `&str`.** `/proc/cpuinfo`, `/proc/meminfo`,
   `/etc/os-release`, `/etc/group` and `--version` output are all parsed by
   functions that take text and return a value. No I/O to mock.
2. **Every filesystem probe goes through a `Sysroot`.** Detection addresses
   files by their canonical absolute path (`/proc/cpuinfo`,
   `/usr/share/OVMF/...`) but resolves them through an injectable root, so a
   test can point the same code at a fixture tree.

```rust
let root = Sysroot::at("tests/fixtures/arch-with-ovmf");
let report = HostReport::detect_in(&root);
```

---

### Step 2 — `daholyvm create`

```
daholyvm create win11 --iso ~/ISOs/Win11.iso --cpus 6 --memory 8192 --disk 100
```

What happens, in order — and the order matters, because **a half-created VM is
worse than none**, so everything that can fail cheaply fails before anything is
written:

1. **Validate the name.** `VmName::new` rejects anything that cannot safely
   become a directory name. This is a *validated newtype*, not a sanitiser:
   quietly rewriting `../../etc` into something harmless would mean the VM you
   asked for and the VM you got have different names. Rejecting says so.
2. **Validate the configuration.** Pure bounds checks — no filesystem access, no
   host inspection. See the [configuration reference](#configuration-reference).
3. **Check the VM does not already exist.** An existing directory is an error,
   never an overwrite.
4. **Locate `qemu-img`,** from the preflight report, by resolved path.
5. **Create the VM directory** under `$XDG_DATA_HOME/daholyvm/vms/<name>/`.
6. **Create the disk** — `qemu-img create -f qcow2 <path> <size>G`. There is an
   explicit guard against an existing image first, because `qemu-img create`
   truncates without asking, and for a disk image that means destroying a
   Windows installation.
7. **Write `config.toml`.**

Real output:

```
Created `win11`
  4 vCPU, 4096 MiB RAM, 8 GiB disk
  /home/you/.local/share/daholyvm/vms/win11

  Boots from /home/you/ISOs/Win11.iso
  Start the installation with: daholyvm run win11
```

qcow2 allocates lazily, so a 64 GiB disk costs about 200 KB until the guest
starts writing.

---

### Step 3 — `daholyvm run`

1. **Validate the name** and **load `config.toml`**, re-running full validation
   on the way in — the file is hand-editable, so it is not trusted.
2. **Detect the host** and refuse if `can_launch()` is false. QEMU's own
   diagnostics for missing firmware are far worse than the remedies `doctor`
   already has, so the error points there instead.
3. **Check the ISO still exists.** This is deliberately *not* part of
   `VmConfig::validate`, which is pure: an ISO can be moved or unmounted long
   after the VM was created.
4. **Provision the UEFI variable store.** The distribution's `OVMF_VARS` file is
   a shared template under `/usr`. Each VM needs a private writable copy, made
   **once** — it holds the boot order and Secure Boot keys the guest writes, so
   recopying the template on every boot would silently discard them.
5. **Build the command line** — the pure function described below.
6. **Spawn QEMU** with an argument vector, inheriting stdio so the guest's
   diagnostics land in your terminal.
7. **Wait.** QEMU's window *is* the VM; shutting Windows down from inside it is
   how the session ends.

---

### The generated QEMU command line

This is the highest-value surface in the project. `qemu::args::build` is a pure
function — `(VmConfig, HostReport, VmPaths) -> Vec<OsString>` — with no
filesystem access, no process spawning and no printing, so the exact command
line a user would get is asserted in unit tests on a machine with no QEMU
installed. A wrong flag here surfaces as a guest that will not boot, hours
later.

Here is the real output for `win11` with a Secure Boot firmware and KVM
available, exactly as `build` produces it:

```
-name     win11
-machine  q35,smm=on
-global   ICH9-LPC.disable_s3=1
-rtc      base=localtime
-accel    kvm
-cpu      host
-smp      4
-m        4096
-global   driver=cfi.pflash01,property=secure,value=on
-drive    if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd
-drive    if=pflash,format=raw,unit=1,file=~/.local/share/daholyvm/vms/win11/OVMF_VARS.fd
-device   ich9-ahci,id=sata
-drive    id=hd,if=none,format=qcow2,file=~/.local/share/daholyvm/vms/win11/disk.qcow2
-device   ide-hd,drive=hd,bus=sata.0
-drive    id=cd,if=none,format=raw,media=cdrom,readonly=on,file=~/ISOs/Win11.iso
-device   ide-cd,drive=cd,bus=sata.1
-boot     order=dc
-netdev   user,id=net0
-device   e1000e,netdev=net0
-device   qemu-xhci,id=usb
-device   usb-tablet,bus=usb.0
-vga      std
```

Every one of those is a decision, and each is the difference between an
installer that runs and one that stops on a black screen or an empty disk list:

| Flag | Why |
| --- | --- |
| `-machine q35` | OVMF and modern Windows both expect it. The older i440fx has no PCIe and no SMM. |
| `smm=on` | System Management Mode is what stops the guest writing its own Secure Boot variables. Without it, Secure Boot is decorative. Set to `off` when the firmware is not Secure Boot capable. |
| `-global ICH9-LPC.disable_s3=1` | Windows guests under QEMU hang rather than resume from S3, and the guest offers "Sleep" in its own menus, so the state is genuinely reachable. |
| `-rtc base=localtime` | Windows keeps the hardware clock in local time and would otherwise drift by your timezone offset on every boot. |
| `-accel kvm` / `-cpu host` | Hardware acceleration when `/dev/kvm` is usable. Falls back to `-accel tcg` / `-cpu qemu64` otherwise — `-cpu host` is meaningless under TCG, where nothing is passed through. |
| `-global driver=cfi.pflash01,property=secure,value=on` | Emitted only for a Secure Boot firmware; makes the variable store honour SMM protection. |
| `-drive if=pflash,unit=0,readonly=on` | The firmware image itself, mounted read-only. |
| `-drive if=pflash,unit=1` | **This VM's private** variable store — never the shared template. |
| `-device ich9-ahci` + `ide-hd` | Emulated SATA, not virtio. The Windows installer ships no virtio driver and would present a disk selection screen listing no disks. See [ADR 0005](docs/adr/0005-emulated-ahci-not-virtio-for-the-system-disk.md). |
| `-device ide-cd` + `-boot order=dc` | The medium is tried first, then the disk, so the same command line installs Windows and then boots what it installed. Without an ISO, `-boot order=c`. |
| `-device e1000e` | Emulated rather than fast, but Windows has the driver in the box, so networking works *during installation*. |
| `-device usb-tablet` | Reports absolute pointer coordinates. Without it the host and guest cursors drift apart and the window has to grab your mouse. |
| `-vga std` | The most broadly compatible adapter for a guest with no drivers installed yet. |

#### One detail worth calling out

QEMU splits `-drive` values on commas. A path like `/iso/Windows 11, 24H2.iso`
would have the rest of its filename read as further options, and QEMU would
reject the lot. So commas are **doubled** during escaping, and paths are handled
as raw bytes throughout, because a filename need not be valid UTF-8:

```
file=/iso/Windows 11,, 24H2.iso
```

There is a test for exactly this, and another asserting that a path containing
spaces, `;` and `$()` survives as one inert argument.

---

## On-disk layout

Everything lives under the XDG data directory — `$XDG_DATA_HOME` when set,
otherwise `~/.local/share`:

```
~/.local/share/daholyvm/
└── vms/
    ├── win11/
    │   ├── config.toml      the VmConfig, hand-editable
    │   ├── disk.qcow2       the system disk
    │   └── OVMF_VARS.fd     this VM's private UEFI variable store
    └── win10/
        └── ...
```

Grouped by VM rather than by file type, so a guest can be backed up, copied or
deleted as a single directory, and it is obvious what belongs to what.

`config.toml` is written by `create` and read by `run`:

```toml
name = "win11"
cpus = 4
memory_mib = 4096
disk_gib = 8
iso = "/home/you/ISOs/Win11.iso"
```

Editing it by hand is supported and expected. It is re-validated on every load,
and unknown keys are an error rather than a silent default — a typo like
`memory_mb` tells you so instead of quietly giving you 4 GiB.

---

## Command reference

### `daholyvm doctor [--json]`

Check whether this host can run Windows virtual machines.

| | |
| --- | --- |
| `--json` | Emit the full report as JSON instead of the checklist |
| Exit `0` | The host can launch a VM |
| Exit `1` | At least one required component is missing |

### `daholyvm create <name> [options]`

Create a new virtual machine.

| Option | Default | Meaning |
| --- | --- | --- |
| `<name>` | — | Name of the VM; becomes its directory name |
| `--iso <PATH>` | none | Installation medium, attached as a CD-ROM |
| `--cpus <N>` | `4` | Virtual CPUs |
| `--memory <MIB>` | `4096` | Memory in MiB |
| `--disk <GIB>` | `64` | Disk size in GiB |

### `daholyvm run <name>`

Boot a VM and wait for it to shut down. Exits `0` when QEMU exits cleanly, `1`
otherwise.

### `daholyvm list`

List the VMs on this system with their sizing.

### Errors

Every failure is one line on stderr naming what to do, and exit code `1`:

```
daholyvm: no virtual machine named `ghost`
daholyvm: a virtual machine named `win11` already exists
daholyvm: invalid virtual machine name `../../etc`: must not start with `.`
daholyvm: invalid configuration: `memory_mib` must be at least 512 MiB
daholyvm: this host cannot launch a virtual machine: run `daholyvm doctor` to see what is missing
daholyvm: installation medium `/iso/gone.iso` does not exist
daholyvm: refusing to overwrite the existing disk image at `...`
```

---

## Configuration reference

### `name`

| | |
| --- | --- |
| Type | string |
| Length | 1–64 characters |
| Allowed | ASCII letters, digits, `-`, `_`, `.` |
| Rejected | anything starting with `.`, path separators, whitespace, shell metacharacters |

Rejected rather than sanitised, so the VM you asked for and the VM you got
always have the same name.

### `cpus`

| | |
| --- | --- |
| Type | integer |
| Default | `4` |
| Range | 1–255 |

### `memory_mib`

| | |
| --- | --- |
| Type | integer, MiB |
| Default | `4096` |
| Minimum | 512 |

### `disk_gib`

| | |
| --- | --- |
| Type | integer, GiB |
| Default | `64` |
| Range | 1–8192 |

### `iso`

| | |
| --- | --- |
| Type | optional path |
| Default | unset |

Attached as a read-only CD-ROM for as long as it is set, and made the first boot
device. Its existence is checked at launch, not at validation, because an ISO
can be moved long after the VM was created.

The defaults are sized for a Windows 11 guest deliberately: a guest that is too
small fails during installation, long after the user has stopped paying
attention. The *bounds*, by contrast, are only what QEMU can be asked for —
whether 512 MiB is *enough* for the guest you have in mind is your call.

---

## Design decisions

Recorded in [`docs/adr/`](docs/adr/), and worth reading before changing
anything structural:

| ADR | Decision |
| --- | --- |
| [0001](docs/adr/0001-orchestration-not-hypervisor.md) | Orchestrate QEMU/KVM/OVMF rather than implement virtualization |
| [0002](docs/adr/0002-invoke-qemu-directly-not-libvirt.md) | Drive QEMU directly instead of going through libvirt |
| [0003](docs/adr/0003-argument-vectors-never-shell-strings.md) | Build argument vectors, never shell strings |
| [0004](docs/adr/0004-preflight-reports-remedies-not-booleans.md) | Preflight reports remedies, not booleans |
| [0005](docs/adr/0005-emulated-ahci-not-virtio-for-the-system-disk.md) | Emulated AHCI for the system disk, not virtio |

The overall design is described in
[`docs/architecture.md`](docs/architecture.md).

---

## Safety posture

- QEMU is invoked with an **argument vector**, never a shell string. User input
  (ISO paths, VM names, sizes) becomes a single `OsString` argument and can
  never be reinterpreted as syntax.
- No user-provided string is ever executed as a command. The only executables
  DA-HOLY-VM runs are `qemu-system-x86_64` and `qemu-img`, resolved from `PATH`
  and version-checked.
- VM names are **validated**, not sanitised, before being used as filesystem
  paths, and the storage layout is asserted to keep every VM directory under
  `vms/`.
- `qemu-img create` is guarded by an explicit existence check, because it
  truncates without asking.
- Nothing runs as root and nothing installs packages. The tool tells you the
  command; you decide whether to run it.
- Discovered binaries are reported by resolved path, because `PATH` shadowing by
  unrelated SDKs is a real and confusing failure mode.

---

## Testing

```
cargo test              # 80 tests, none need QEMU, KVM or firmware installed
cargo clippy --all-targets
cargo fmt
```

The design makes this possible rather than a mock framework:

- **Parsers are pure functions over `&str`** — `/proc/cpuinfo`, `/proc/meminfo`,
  `/etc/os-release`, `/etc/group`, `--version` output.
- **Filesystem probes go through `Sysroot`**, pointed at fixture trees.
- **`qemu::args::build` is pure**, so the entire command line is asserted
  without ever launching a VM. Twelve tests cover acceleration fallback, Secure
  Boot, boot order with and without an ISO, comma escaping, and the absence of
  virtio block devices.
- **`VmConfig::validate` is pure**, and storage goes through an injectable root,
  so VM creation and loading are tested inside a temporary directory.
- **Process handling is tested against `/bin/sh`**, which stands in for QEMU:
  the point is that the wrapper reports a real exit status rather than
  swallowing it.

Every commit that adds Rust code since the workspace was scaffolded compiles on
its own — verified by checking each one out in a scratch clone. The two earliest
commits (the empty workspace, and the core crate before the CLI's `main.rs`
existed) do not build as a whole workspace, which is what a scaffolding commit
looks like.

---

## What does not work yet

Read this before assuming a Windows 11 install will succeed.

- **No TPM.** Windows 11 checks for TPM 2.0 during setup and refuses to install
  without it. Windows 10 guests are unaffected. Emulating one means driving
  `swtpm` as a second process with its own socket, which is the next thing to
  build.
- **No end-to-end boot has been verified.** The development host has neither
  `qemu-system-x86_64` nor OVMF installed, so `run` correctly refuses before
  spawning anything. The command line is unit tested; it has not yet installed
  Windows.
- **Storage is emulated AHCI, not virtio.** Deliberate — see
  [ADR 0005](docs/adr/0005-emulated-ahci-not-virtio-for-the-system-disk.md) —
  but it costs disk throughput.
- **Stopping a VM from outside it is a power cut.** Shutting down from inside
  the guest is clean. A graceful external stop needs QEMU's QMP socket, so
  there is no `daholyvm stop` yet.
- **`run` blocks.** VMs cannot be backgrounded or managed while running, for the
  same reason.
- **No delete, rename, snapshot or resize.** Deleting a VM today means removing
  its directory.
- **No `--dry-run`** to print the command line without booting.
- **x86_64 only,** and there are no plans to change that.

---

## Roadmap

**Next:** `swtpm` integration for TPM 2.0, which is what stands between this and
a working Windows 11 install.

**After that:** the QMP monitor socket, which unlocks graceful shutdown, VM
status, and background VMs in one go — `stop`, `list --running`, and a `run`
that returns.

**Then:** `delete`, snapshots, a virtio-win path for post-install performance,
and the GUI over the same core types.

---

## Troubleshooting

**`doctor` says `qemu-img` is older than 6.0.0, but I installed QEMU.**
Something else on your `PATH` is shadowing it — the Android SDK is the usual
culprit. Run `which -a qemu-img`; the first hit wins. `doctor` shows you the
resolved path for exactly this reason.

**`doctor` says KVM permission denied.**
You are not in the owning group. `doctor` names it; usually
`sudo usermod -aG kvm "$USER"`, then log out and back in — group membership is
established at login.

**`run` says the host cannot launch a virtual machine.**
Run `daholyvm doctor`. Something is `Missing`, and the report says which and how
to fix it.

**Windows 11 setup says this PC doesn't meet the requirements.**
That is the missing TPM. Not yet supported — see
[What does not work yet](#what-does-not-work-yet).

**The installer shows no disks.**
Should not happen, since storage is AHCI specifically to avoid it. If it does,
check that `-device ich9-ahci` is in the command line and file a bug.

**The guest clock is off by my timezone.**
Should not happen — `-rtc base=localtime` is always set. Check the guest's own
time settings.

---

## Repository layout

```
crates/
  daholyvm-core/          all domain logic; no GUI or CLI dependencies
    src/
      config.rs           VmConfig, VmName, validation, TOML
      paths.rs            XDG storage layout
      disk.rs             qcow2 creation via qemu-img
      vm.rs               create / load / launch lifecycle
      error.rs            one error type for the crate
      qemu/
        args.rs           the pure command-line builder
        runtime.rs        the QEMU child process
      preflight/          host capability detection
        cpu.rs kvm.rs qemu.rs firmware.rs distro.rs platform.rs
        sysroot.rs        injectable filesystem root
        which.rs          minimal which(1)
  daholyvm-cli/           thin `daholyvm` binary
    src/
      main.rs             argument parsing, exit codes
      render.rs           human-readable output
docs/
  architecture.md         the design, and why
  adr/                    decisions that are expensive to revisit
```

### Development

```
cargo test
cargo clippy --all-targets
cargo fmt
cargo run -p daholyvm-cli -- doctor
```

Requires Rust 1.74 or newer. Licensed MIT OR Apache-2.0.
