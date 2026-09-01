//! The description of a virtual machine, and the rules for a valid one.
//!
//! A `VmConfig` is the whole of what DA-HOLY-VM knows about a guest. It is
//! persisted as TOML next to the VM's disk so that it stays readable and
//! hand-editable — a config file the user cannot inspect is a config file they
//! cannot debug.
//!
//! Validation here is **pure**: it inspects the values and nothing else, never
//! the filesystem or the host. Whether the host can satisfy a config is a
//! separate question, answered by [`crate::preflight`], and whether a path
//! exists is checked at launch. Keeping those apart is what allows the whole
//! rule set to be unit tested.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Defaults sized for a Windows 11 guest, which wants 4 GiB of RAM and 64 GiB
/// of disk. They are deliberately generous: a guest that is too small fails
/// during installation, long after the user has stopped paying attention.
pub const DEFAULT_CPUS: u32 = 4;
pub const DEFAULT_MEMORY_MIB: u64 = 4096;
pub const DEFAULT_DISK_GIB: u64 = 64;

/// Longest name accepted, comfortably inside the 255 byte limit every Linux
/// filesystem imposes on a single path component.
pub const MAX_NAME_LEN: usize = 64;

const MAX_CPUS: u32 = 255;
const MIN_MEMORY_MIB: u64 = 512;
const MIN_DISK_GIB: u64 = 1;
const MAX_DISK_GIB: u64 = 8192;

/// A virtual machine name that is safe to use as a directory name.
///
/// Names reach the filesystem, so they are validated rather than sanitised:
/// silently rewriting `../../etc` into something harmless would mean the VM the
/// user asked for and the VM they got have different names. Rejecting says so.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct VmName(String);

impl VmName {
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let reject = |reason| {
            Err(Error::InvalidName {
                name: name.clone(),
                reason,
            })
        };

        if name.is_empty() {
            return reject("a name is required");
        }
        if name.len() > MAX_NAME_LEN {
            return reject("longer than 64 characters");
        }
        // `.` and `..` are directory traversal; a leading dot merely hides the
        // VM from `ls`, which is confusing rather than dangerous, but neither
        // is worth supporting.
        if name.starts_with('.') {
            return reject("must not start with `.`");
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return reject("only letters, digits, `-`, `_` and `.` are allowed");
        }

        Ok(VmName(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VmName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for VmName {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        VmName::new(value)
    }
}

impl From<VmName> for String {
    fn from(name: VmName) -> String {
        name.0
    }
}

/// Everything DA-HOLY-VM knows about one guest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmConfig {
    pub name: VmName,
    #[serde(default = "default_cpus")]
    pub cpus: u32,
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u64,
    #[serde(default = "default_disk_gib")]
    pub disk_gib: u64,
    /// Installation medium, attached as a CD-ROM for as long as it is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iso: Option<PathBuf>,
}

fn default_cpus() -> u32 {
    DEFAULT_CPUS
}

fn default_memory_mib() -> u64 {
    DEFAULT_MEMORY_MIB
}

fn default_disk_gib() -> u64 {
    DEFAULT_DISK_GIB
}

impl VmConfig {
    /// A new VM with default sizing.
    pub fn new(name: VmName) -> Self {
        VmConfig {
            name,
            cpus: DEFAULT_CPUS,
            memory_mib: DEFAULT_MEMORY_MIB,
            disk_gib: DEFAULT_DISK_GIB,
            iso: None,
        }
    }

    pub fn with_iso(mut self, iso: Option<PathBuf>) -> Self {
        self.iso = iso;
        self
    }

    /// Check the values against the range QEMU can actually be asked for.
    ///
    /// These are hard limits, not advice. Whether 512 MiB is *enough* for the
    /// guest the user has in mind is their call; whether QEMU will accept it
    /// is not.
    pub fn validate(&self) -> Result<()> {
        let invalid = |field, problem: String| Err(Error::InvalidConfig { field, problem });

        if self.cpus == 0 {
            return invalid("cpus", "must be at least 1".to_owned());
        }
        if self.cpus > MAX_CPUS {
            return invalid("cpus", format!("must be at most {MAX_CPUS}"));
        }
        if self.memory_mib < MIN_MEMORY_MIB {
            return invalid(
                "memory_mib",
                format!("must be at least {MIN_MEMORY_MIB} MiB"),
            );
        }
        if self.disk_gib < MIN_DISK_GIB {
            return invalid("disk_gib", format!("must be at least {MIN_DISK_GIB} GiB"));
        }
        if self.disk_gib > MAX_DISK_GIB {
            return invalid("disk_gib", format!("must be at most {MAX_DISK_GIB} GiB"));
        }

        Ok(())
    }

    /// Parse and validate a saved configuration.
    pub fn from_toml(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let config: VmConfig = toml::from_str(text).map_err(|source| Error::ConfigSyntax {
            path: path.into(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> VmName {
        VmName::new(text).expect("valid name")
    }

    #[test]
    fn accepts_ordinary_names() {
        for text in ["win11", "Windows-11", "work_vm", "vm.2"] {
            assert_eq!(name(text).as_str(), text);
        }
    }

    #[test]
    fn rejects_path_separators_and_traversal() {
        for text in ["..", ".", "../etc", "a/b", "/abs", "."] {
            assert!(
                VmName::new(text).is_err(),
                "`{text}` must not be accepted as a name"
            );
        }
    }

    #[test]
    fn rejects_shell_and_whitespace_characters() {
        for text in ["a b", "a;b", "$(id)", "a\nb", "a*"] {
            assert!(VmName::new(text).is_err(), "`{text}` must be rejected");
        }
    }

    #[test]
    fn rejects_empty_and_overlong_names() {
        assert!(VmName::new("").is_err());
        assert!(VmName::new("a".repeat(MAX_NAME_LEN)).is_ok());
        assert!(VmName::new("a".repeat(MAX_NAME_LEN + 1)).is_err());
    }

    #[test]
    fn defaults_are_valid() {
        assert!(VmConfig::new(name("win11")).validate().is_ok());
    }

    #[test]
    fn rejects_sizes_qemu_cannot_be_asked_for() {
        let base = VmConfig::new(name("win11"));

        let mut zero_cpus = base.clone();
        zero_cpus.cpus = 0;
        assert!(zero_cpus.validate().is_err());

        let mut tiny = base.clone();
        tiny.memory_mib = 64;
        assert!(tiny.validate().is_err());

        let mut no_disk = base;
        no_disk.disk_gib = 0;
        assert!(no_disk.validate().is_err());
    }

    #[test]
    fn survives_a_toml_round_trip() {
        let config = VmConfig::new(name("win11")).with_iso(Some(PathBuf::from("/iso/Win11.iso")));
        let text = config.to_toml().unwrap();
        assert_eq!(VmConfig::from_toml(&text, "config.toml").unwrap(), config);
    }

    #[test]
    fn omitted_fields_take_their_defaults() {
        let config = VmConfig::from_toml("name = \"win11\"\n", "config.toml").unwrap();
        assert_eq!(config.cpus, DEFAULT_CPUS);
        assert_eq!(config.memory_mib, DEFAULT_MEMORY_MIB);
        assert_eq!(config.disk_gib, DEFAULT_DISK_GIB);
        assert_eq!(config.iso, None);
    }

    #[test]
    fn a_misspelled_field_is_an_error_not_a_silent_default() {
        let text = "name = \"win11\"\nmemory_mb = 8192\n";
        let err = VmConfig::from_toml(text, "config.toml").unwrap_err();
        assert!(
            err.to_string().contains("memory_mb"),
            "the unknown key must be named: {err}"
        );
    }

    #[test]
    fn an_unsafe_name_is_rejected_when_loading_too() {
        let text = "name = \"../../etc\"\n";
        assert!(VmConfig::from_toml(text, "config.toml").is_err());
    }

    #[test]
    fn validation_runs_on_load() {
        let text = "name = \"win11\"\ncpus = 0\n";
        assert!(VmConfig::from_toml(text, "config.toml").is_err());
    }
}
