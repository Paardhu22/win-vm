//! Distribution identification, used only to phrase install instructions.
//!
//! DA-HOLY-VM never runs a package manager itself; it tells the user the exact
//! command to run. Guessing wrong is harmless, so unknown distributions fall
//! back to naming the upstream project instead of a package.

use serde::Serialize;

use super::Sysroot;

/// The subset of `/etc/os-release` we care about.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OsRelease {
    pub id: Option<String>,
    pub id_like: Vec<String>,
    pub pretty_name: Option<String>,
}

impl OsRelease {
    pub fn detect_in(root: &Sysroot) -> Self {
        root.read("/etc/os-release")
            .as_deref()
            .map(parse_os_release)
            .unwrap_or_default()
    }

    /// Resolve to a package manager, consulting `ID_LIKE` so that derivatives
    /// (Manjaro, Linux Mint, Nobara, ...) inherit their parent's commands.
    pub fn package_manager(&self) -> PackageManager {
        let ids = self
            .id
            .iter()
            .chain(self.id_like.iter())
            .map(String::as_str);
        for id in ids {
            match id {
                "arch" | "archarm" | "manjaro" | "endeavouros" | "cachyos" => {
                    return PackageManager::Pacman
                }
                "debian" | "ubuntu" | "linuxmint" | "pop" | "raspbian" => {
                    return PackageManager::Apt
                }
                "fedora" | "rhel" | "centos" | "nobara" | "bazzite" => return PackageManager::Dnf,
                "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" | "sles" | "suse" => {
                    return PackageManager::Zypper
                }
                _ => {}
            }
        }
        PackageManager::Unknown
    }
}

/// Parse the shell-like `KEY=value` format of `/etc/os-release`.
pub fn parse_os_release(text: &str) -> OsRelease {
    let mut out = OsRelease::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "ID" => out.id = Some(value),
            "ID_LIKE" => {
                out.id_like = value.split_whitespace().map(str::to_owned).collect();
            }
            "PRETTY_NAME" => out.pretty_name = Some(value),
            _ => {}
        }
    }
    out
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

/// A third-party component DA-HOLY-VM depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package {
    Qemu,
    Ovmf,
    /// The TPM 2.0 emulator, without which Windows 11 setup refuses to run.
    Swtpm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Pacman,
    Apt,
    Dnf,
    Zypper,
    Unknown,
}

impl PackageManager {
    /// The exact command the user should run to obtain `package`.
    pub fn install_hint(self, package: Package) -> String {
        let name = match (self, package) {
            (PackageManager::Pacman, Package::Qemu) => "qemu-desktop",
            (PackageManager::Pacman, Package::Ovmf) => "edk2-ovmf",
            (PackageManager::Pacman, Package::Swtpm) => "swtpm",
            (PackageManager::Apt, Package::Qemu) => "qemu-system-x86",
            (PackageManager::Apt, Package::Ovmf) => "ovmf",
            (PackageManager::Apt, Package::Swtpm) => "swtpm",
            (PackageManager::Dnf, Package::Qemu) => "qemu-system-x86",
            (PackageManager::Dnf, Package::Ovmf) => "edk2-ovmf",
            (PackageManager::Dnf, Package::Swtpm) => "swtpm",
            (PackageManager::Zypper, Package::Qemu) => "qemu-x86",
            (PackageManager::Zypper, Package::Ovmf) => "qemu-ovmf-x86_64",
            (PackageManager::Zypper, Package::Swtpm) => "swtpm",
            (PackageManager::Unknown, Package::Qemu) => {
                return "install QEMU (the `qemu-system-x86_64` binary) using your distribution's \
                        package manager"
                    .to_owned()
            }
            (PackageManager::Unknown, Package::Ovmf) => {
                return "install the OVMF/edk2 UEFI firmware package using your distribution's \
                        package manager"
                    .to_owned()
            }
            (PackageManager::Unknown, Package::Swtpm) => {
                return "install the `swtpm` TPM emulator using your distribution's package \
                        manager"
                    .to_owned()
            }
        };
        match self {
            PackageManager::Pacman => format!("sudo pacman -S --needed {name}"),
            PackageManager::Apt => format!("sudo apt install {name}"),
            PackageManager::Dnf => format!("sudo dnf install {name}"),
            PackageManager::Zypper => format!("sudo zypper install {name}"),
            PackageManager::Unknown => unreachable!("handled above"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_and_bare_values() {
        let os = parse_os_release(
            "NAME=\"Arch Linux\"\nID=arch\nPRETTY_NAME=\"Arch Linux\"\nBUILD_ID=rolling\n",
        );
        assert_eq!(os.id.as_deref(), Some("arch"));
        assert_eq!(os.pretty_name.as_deref(), Some("Arch Linux"));
        assert!(os.id_like.is_empty());
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let os = parse_os_release("# comment\n\nID=fedora\n");
        assert_eq!(os.id.as_deref(), Some("fedora"));
    }

    #[test]
    fn derivatives_inherit_parent_package_manager() {
        let os = parse_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n");
        assert_eq!(os.package_manager(), PackageManager::Apt);
    }

    #[test]
    fn unknown_distro_still_produces_actionable_text() {
        let os = parse_os_release("ID=plan9\n");
        assert_eq!(os.package_manager(), PackageManager::Unknown);
        let hint = os.package_manager().install_hint(Package::Qemu);
        assert!(hint.contains("qemu-system-x86_64"), "hint was: {hint}");
    }

    #[test]
    fn known_distro_hints_name_a_real_command() {
        assert_eq!(
            PackageManager::Pacman.install_hint(Package::Ovmf),
            "sudo pacman -S --needed edk2-ovmf"
        );
        assert_eq!(
            PackageManager::Apt.install_hint(Package::Qemu),
            "sudo apt install qemu-system-x86"
        );
    }
}
