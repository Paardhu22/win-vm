//! A relocatable view of the filesystem.
//!
//! Detection code addresses files by their canonical absolute location
//! (`/proc/cpuinfo`, `/usr/share/OVMF/...`). Routing every one of those reads
//! through a `Sysroot` lets the test suite point the same code at a fixture
//! tree, so preflight is testable on a host that has none of the real files.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Sysroot {
    root: PathBuf,
}

impl Sysroot {
    /// The real running system.
    pub fn host() -> Self {
        Sysroot {
            root: PathBuf::from("/"),
        }
    }

    /// A fixture tree standing in for the system, used by tests.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Sysroot { root: root.into() }
    }

    /// Rebase a canonical absolute path onto this root.
    pub fn resolve(&self, absolute: &str) -> PathBuf {
        self.root.join(absolute.trim_start_matches('/'))
    }

    pub fn exists(&self, absolute: &str) -> bool {
        self.resolve(absolute).exists()
    }

    /// Read a file, treating any failure as absence. Preflight is best-effort:
    /// an unreadable `/proc` entry should degrade the report, not abort it.
    pub fn read(&self, absolute: &str) -> Option<String> {
        std::fs::read_to_string(self.resolve(absolute)).ok()
    }

    /// True when this sysroot is the real filesystem, which is the only case
    /// where probing live kernel interfaces is meaningful.
    pub fn is_host(&self) -> bool {
        self.root == Path::new("/")
    }
}

impl Default for Sysroot {
    fn default() -> Self {
        Sysroot::host()
    }
}
