//! CPU facts, in particular whether hardware virtualization extensions exist.

use serde::Serialize;

use super::{Requirement, Status, Sysroot};

/// Hardware virtualization extension advertised by the CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum VirtExtension {
    /// Intel VT-x, reported as the `vmx` flag.
    IntelVtx,
    /// AMD-V, reported as the `svm` flag.
    AmdV,
}

impl VirtExtension {
    pub fn label(self) -> &'static str {
        match self {
            VirtExtension::IntelVtx => "Intel VT-x",
            VirtExtension::AmdV => "AMD-V",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Cpu {
    pub model: Option<String>,
    pub logical_cores: usize,
    /// `None` means the CPU does not advertise virtualization extensions, which
    /// on a physical machine usually means they are disabled in firmware.
    pub virt: Option<VirtExtension>,
}

impl Cpu {
    pub fn detect_in(root: &Sysroot) -> Self {
        parse_cpuinfo(&root.read("/proc/cpuinfo").unwrap_or_default())
    }

    pub(crate) fn requirement(&self) -> Requirement {
        let cores = self.logical_cores;
        let model = self.model.as_deref().unwrap_or("unknown CPU");
        match self.virt {
            Some(ext) => Requirement::new(
                "cpu.virtualization",
                "CPU virtualization extensions",
                Status::Ok,
                format!("{} present ({model}, {cores} logical cores)", ext.label()),
            ),
            None => Requirement::new(
                "cpu.virtualization",
                "CPU virtualization extensions",
                Status::Warn,
                format!("neither `vmx` nor `svm` reported by {model}"),
            )
            .with_remedy(
                "enable Intel VT-x / AMD-V (often listed as \"Virtualization Technology\", \"SVM\" \
                 or \"Intel VMX\") in your BIOS/UEFI setup; on a nested or cloud guest, ask your \
                 provider to enable nested virtualization",
            ),
        }
    }
}

/// Parse `/proc/cpuinfo`. Only the first processor block is used for the model
/// and flags; every block is counted for the logical core total.
pub fn parse_cpuinfo(text: &str) -> Cpu {
    let mut model = None;
    let mut virt = None;
    let mut logical_cores = 0usize;

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "processor" => logical_cores += 1,
            "model name" if model.is_none() => model = Some(value.to_owned()),
            "flags" | "Features" if virt.is_none() => {
                virt = value.split_whitespace().find_map(|flag| match flag {
                    "vmx" => Some(VirtExtension::IntelVtx),
                    "svm" => Some(VirtExtension::AmdV),
                    _ => None,
                });
            }
            _ => {}
        }
    }

    Cpu {
        model,
        logical_cores,
        virt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_intel_vt_x_and_counts_cores() {
        let cpu = parse_cpuinfo(
            "processor\t: 0\nmodel name\t: 13th Gen Intel(R) Core(TM) i7-13620H\nflags\t\t: fpu vme vmx smx est\n\
             processor\t: 1\nmodel name\t: 13th Gen Intel(R) Core(TM) i7-13620H\nflags\t\t: fpu vme vmx smx est\n",
        );
        assert_eq!(cpu.virt, Some(VirtExtension::IntelVtx));
        assert_eq!(cpu.logical_cores, 2);
        assert!(cpu.model.unwrap().contains("i7-13620H"));
    }

    #[test]
    fn detects_amd_v() {
        let cpu = parse_cpuinfo("processor\t: 0\nflags\t\t: fpu tsc svm cr8_legacy\n");
        assert_eq!(cpu.virt, Some(VirtExtension::AmdV));
    }

    #[test]
    fn absent_extensions_warn_and_explain_how_to_enable_them() {
        let cpu = parse_cpuinfo("processor\t: 0\nmodel name\t: Some CPU\nflags\t\t: fpu tsc\n");
        assert_eq!(cpu.virt, None);
        let req = cpu.requirement();
        assert_eq!(req.status, Status::Warn);
        assert!(req.remedy.unwrap().contains("BIOS/UEFI"));
    }

    #[test]
    fn substring_matches_do_not_count_as_flags() {
        // `vmxnet3` is a driver name, not the VT-x flag.
        let cpu = parse_cpuinfo("processor\t: 0\nflags\t\t: fpu vmxnet3 svmalike\n");
        assert_eq!(cpu.virt, None);
    }

    #[test]
    fn empty_cpuinfo_does_not_panic() {
        let cpu = parse_cpuinfo("");
        assert_eq!(cpu.logical_cores, 0);
        assert_eq!(cpu.virt, None);
    }
}
