//! Hardware write paths. Unlike the rest of the crate these functions modify
//! system state, so every value is validated against the kernel-advertised set
//! before it is written and callers must hold root. Only the intel_pstate/cpufreq
//! surface is implemented here — it is verified on this machine. Fan, RGB and
//! Acer thermal-profile control are intentionally absent until their firmware
//! interfaces are confirmed.

use crate::sysfs::read_trim;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CPU_BASE: &str = "/sys/devices/system/cpu";
const NO_TURBO: &str = "/sys/devices/system/cpu/intel_pstate/no_turbo";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBackup {
    pub cpu: String,
    pub governor: Option<String>,
    pub epp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuBackup {
    pub policies: Vec<PolicyBackup>,
    pub no_turbo: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CpuStatus {
    pub driver: Option<String>,
    pub governor: Option<String>,
    pub epp: Option<String>,
    pub turbo_enabled: Option<bool>,
    pub available_governors: Vec<String>,
    pub available_epp: Vec<String>,
}

pub fn status() -> CpuStatus {
    let base = format!("{CPU_BASE}/cpu0/cpufreq");
    CpuStatus {
        driver: read_trim(format!("{base}/scaling_driver")),
        governor: read_trim(format!("{base}/scaling_governor")),
        epp: read_trim(format!("{base}/energy_performance_preference")),
        turbo_enabled: read_trim(NO_TURBO).map(|v| v == "0"),
        available_governors: available_governors(),
        available_epp: available_epp(),
    }
}

pub fn available_governors() -> Vec<String> {
    list(format!("{CPU_BASE}/cpu0/cpufreq/scaling_available_governors"))
}

pub fn available_epp() -> Vec<String> {
    list(format!("{CPU_BASE}/cpu0/cpufreq/energy_performance_available_preferences"))
}

/// Snapshot the current governor/EPP/turbo so `restore` can undo `apply_*`.
pub fn capture() -> CpuBackup {
    let policies = policy_dirs()
        .into_iter()
        .map(|dir| PolicyBackup {
            cpu: cpu_name(&dir),
            governor: read_trim(dir.join("scaling_governor")),
            epp: read_trim(dir.join("energy_performance_preference")),
        })
        .collect();
    CpuBackup {
        policies,
        no_turbo: read_trim(NO_TURBO),
    }
}

pub fn apply_performance() -> io::Result<()> {
    let govs = available_governors();
    for dir in policy_dirs() {
        set_governor(&dir, "performance", &govs)?;
    }
    set_turbo(true)
}

pub fn apply_balanced() -> io::Result<()> {
    let govs = available_governors();
    let epps = available_epp();
    for dir in policy_dirs() {
        set_governor(&dir, "powersave", &govs)?;
        // EPP is only writable under the powersave governor; treat as best effort.
        set_epp(&dir, "balance_performance", &epps);
    }
    set_turbo(true)
}

/// Apply an arbitrary cpufreq lever set to every policy. `governor` is validated
/// against the kernel-advertised list; `epp` is best-effort (only honoured under
/// the powersave governor in intel_pstate active mode). Turbo maps to no_turbo.
pub fn apply(governor: &str, epp: Option<&str>, turbo: bool) -> io::Result<()> {
    let govs = available_governors();
    let epps = available_epp();
    for dir in policy_dirs() {
        set_governor(&dir, governor, &govs)?;
        if let Some(e) = epp {
            set_epp(&dir, e, &epps);
        }
    }
    set_turbo(turbo)
}

pub fn restore(backup: &CpuBackup) -> io::Result<()> {
    let govs = available_governors();
    let epps = available_epp();
    for p in &backup.policies {
        let dir = Path::new(CPU_BASE).join(&p.cpu).join("cpufreq");
        if let Some(g) = &p.governor {
            let _ = set_governor(&dir, g, &govs);
        }
        if let Some(e) = &p.epp {
            set_epp(&dir, e, &epps);
        }
    }
    if let Some(nt) = &backup.no_turbo {
        if Path::new(NO_TURBO).exists() {
            let _ = fs::write(NO_TURBO, nt);
        }
    }
    Ok(())
}

fn set_governor(dir: &Path, governor: &str, allowed: &[String]) -> io::Result<()> {
    ensure_allowed(governor, allowed, "governor")?;
    fs::write(dir.join("scaling_governor"), governor)
}

fn set_epp(dir: &Path, epp: &str, allowed: &[String]) {
    let path = dir.join("energy_performance_preference");
    if path.exists() && allowed.iter().any(|a| a == epp) {
        let _ = fs::write(path, epp);
    }
}

fn set_turbo(enabled: bool) -> io::Result<()> {
    if !Path::new(NO_TURBO).exists() {
        return Ok(());
    }
    fs::write(NO_TURBO, if enabled { "0" } else { "1" })
}

fn ensure_allowed(value: &str, allowed: &[String], kind: &str) -> io::Result<()> {
    if allowed.iter().any(|a| a == value) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{kind} '{value}' not available; options: {}", allowed.join(", ")),
        ))
    }
}

fn policy_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(rd) = fs::read_dir(CPU_BASE) {
        for entry in rd.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_cpu_n = name
                .strip_prefix("cpu")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
            if is_cpu_n {
                let cf = entry.path().join("cpufreq");
                if cf.is_dir() {
                    dirs.push(cf);
                }
            }
        }
    }
    dirs.sort();
    dirs
}

fn cpu_name(cpufreq_dir: &Path) -> String {
    cpufreq_dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn list<P: AsRef<Path>>(path: P) -> Vec<String> {
    read_trim(path)
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn govs() -> Vec<String> {
        vec!["performance".into(), "powersave".into()]
    }

    #[test]
    fn allowed_value_passes() {
        assert!(ensure_allowed("performance", &govs(), "governor").is_ok());
    }

    #[test]
    fn disallowed_value_rejected() {
        let err = ensure_allowed("ludicrous", &govs(), "governor").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn empty_allowed_rejects_everything() {
        assert!(ensure_allowed("performance", &[], "governor").is_err());
    }
}
