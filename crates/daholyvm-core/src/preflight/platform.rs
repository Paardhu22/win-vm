//! Host platform facts: distribution, kernel, architecture and memory.

use serde::Serialize;

use super::{OsRelease, PackageManager, Requirement, Status, Sysroot};

/// Architecture this build of DA-HOLY-VM was compiled for.
pub const HOST_ARCH: &str = std::env::consts::ARCH;

#[derive(Debug, Clone, Serialize)]
pub struct Platform {
    /// Whether this build targets Linux at all. DA-HOLY-VM is Linux-first.
    pub is_linux: bool,
    pub arch: &'static str,
    pub kernel: Option<String>,
    pub os_release: OsRelease,
    pub memory_total_kib: Option<u64>,
    pub memory_available_kib: Option<u64>,
}

impl Platform {
    pub fn detect_in(root: &Sysroot) -> Self {
        let meminfo = root.read("/proc/meminfo").unwrap_or_default();
        Platform {
            is_linux: cfg!(target_os = "linux"),
            arch: HOST_ARCH,
            kernel: root
                .read("/proc/sys/kernel/osrelease")
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            os_release: OsRelease::detect_in(root),
            memory_total_kib: parse_meminfo(&meminfo, "MemTotal"),
            memory_available_kib: parse_meminfo(&meminfo, "MemAvailable"),
        }
    }

    pub fn package_manager(&self) -> PackageManager {
        self.os_release.package_manager()
    }

    /// Distribution name for display, falling back to a generic label.
    pub fn distro_name(&self) -> &str {
        self.os_release
            .pretty_name
            .as_deref()
            .or(self.os_release.id.as_deref())
            .unwrap_or("unknown Linux distribution")
    }

    pub(crate) fn requirement(&self) -> Requirement {
        if !self.is_linux {
            return Requirement::new(
                "platform.linux",
                "Linux host",
                Status::Missing,
                format!("this build targets `{}`, not Linux", std::env::consts::OS),
            )
            .with_remedy("DA-HOLY-VM is Linux-first and has no Windows or macOS host support");
        }

        let mut detail = self.distro_name().to_owned();
        if let Some(kernel) = &self.kernel {
            detail.push_str(&format!(", kernel {kernel}"));
        }
        detail.push_str(&format!(", {}", self.arch));

        // A non-x86_64 host would need a different QEMU target and firmware
        // than the MVP builds command lines for.
        if self.arch != "x86_64" {
            return Requirement::new("platform.linux", "Linux host", Status::Warn, detail)
                .with_remedy(
                    "DA-HOLY-VM currently targets x86_64 hosts; running an x86_64 Windows guest \
                     on this architecture would require full emulation and is not supported yet",
                );
        }

        Requirement::new("platform.linux", "Linux host", Status::Ok, detail)
    }
}

/// Extract a `/proc/meminfo` field, in kibibytes.
pub fn parse_meminfo(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        return rest.split_whitespace().next()?.parse().ok();
    }
    None
}

/// Render kibibytes as a human-readable size, e.g. `15.3 GiB`.
pub fn format_kib(kib: u64) -> String {
    const MIB: f64 = 1024.0;
    const GIB: f64 = 1024.0 * 1024.0;
    let kib = kib as f64;
    if kib >= GIB {
        format!("{:.1} GiB", kib / GIB)
    } else if kib >= MIB {
        format!("{:.0} MiB", kib / MIB)
    } else {
        format!("{kib:.0} KiB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str =
        "MemTotal:       16283264 kB\nMemFree:          458220 kB\nMemAvailable:    4812940 kB\n";

    #[test]
    fn reads_named_meminfo_fields() {
        assert_eq!(parse_meminfo(MEMINFO, "MemTotal"), Some(16_283_264));
        assert_eq!(parse_meminfo(MEMINFO, "MemAvailable"), Some(4_812_940));
    }

    #[test]
    fn missing_meminfo_field_is_none_not_zero() {
        assert_eq!(parse_meminfo(MEMINFO, "Hugepagesize"), None);
        assert_eq!(parse_meminfo("", "MemTotal"), None);
    }

    #[test]
    fn does_not_confuse_prefixed_field_names() {
        // `MemFree` must not satisfy a lookup for `Mem`.
        assert_eq!(parse_meminfo(MEMINFO, "Mem"), None);
    }

    #[test]
    fn formats_sizes_at_each_scale() {
        assert_eq!(format_kib(16_283_264), "15.5 GiB");
        assert_eq!(format_kib(524_288), "512 MiB");
        assert_eq!(format_kib(64), "64 KiB");
    }
}
