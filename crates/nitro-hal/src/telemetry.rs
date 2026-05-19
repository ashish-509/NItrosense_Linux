use crate::hwmon::{self, HwmonChip};
use crate::sysfs::{read_i64, read_trim, read_u64};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct Telemetry {
    pub cpu: CpuTelemetry,
    pub gpu: Option<GpuTelemetry>,
    pub battery: Option<BatteryTelemetry>,
    pub fans: Vec<FanReading>,
}

#[derive(Debug, Serialize)]
pub struct CpuTelemetry {
    pub package_temp_c: Option<f64>,
    pub utilization_pct: Option<f64>,
    pub avg_freq_mhz: Option<f64>,
    pub max_freq_mhz: Option<f64>,
    pub power_w: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct GpuTelemetry {
    pub name: String,
    pub temp_c: Option<f64>,
    pub utilization_pct: Option<f64>,
    pub clock_mhz: Option<f64>,
    pub power_w: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct BatteryTelemetry {
    pub capacity_pct: Option<u64>,
    pub status: Option<String>,
    pub power_w: Option<f64>,
    pub ac_online: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct FanReading {
    pub name: String,
    pub rpm: u64,
}

/// Aggregate CPU jiffies from /proc/stat. Utilization requires two samples.
#[derive(Debug, Clone, Copy)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

impl CpuTimes {
    pub fn read() -> Option<CpuTimes> {
        let stat = fs::read_to_string("/proc/stat").ok()?;
        let line = stat.lines().next()?;
        let mut fields = line.split_whitespace();
        if fields.next()? != "cpu" {
            return None;
        }
        let vals: Vec<u64> = fields.filter_map(|v| v.parse().ok()).collect();
        // fields: user nice system idle iowait irq softirq steal guest guest_nice
        let idle = *vals.get(3)? + vals.get(4).copied().unwrap_or(0);
        let total: u64 = vals.iter().sum();
        Some(CpuTimes { idle, total })
    }

    /// Busy percentage between an earlier sample (self) and a later one. Pure.
    pub fn utilization(&self, newer: &CpuTimes) -> Option<f64> {
        let dt = newer.total.checked_sub(self.total)?;
        let di = newer.idle.checked_sub(self.idle)?;
        if dt == 0 {
            return None;
        }
        Some((dt.saturating_sub(di) as f64 / dt as f64) * 100.0)
    }
}

/// One telemetry snapshot. Blocks for `window` to measure CPU utilization and
/// RAPL power deltas; intended for CLI/one-shot use, not the daemon hot path.
pub fn sample(window: Duration) -> Telemetry {
    let cpu0 = CpuTimes::read();
    let energy0 = read_rapl_energy_uj();
    std::thread::sleep(window);
    let cpu1 = CpuTimes::read();
    let energy1 = read_rapl_energy_uj();

    let utilization_pct = match (cpu0, cpu1) {
        (Some(a), Some(b)) => a.utilization(&b),
        _ => None,
    };
    let power_w = match (energy0, energy1) {
        (Some(a), Some(b)) if b >= a => Some((b - a) as f64 / 1e6 / window.as_secs_f64()),
        _ => None,
    };

    let chips = hwmon::read();
    let (avg_freq_mhz, max_freq_mhz) = cpu_freqs();

    Telemetry {
        cpu: CpuTelemetry {
            package_temp_c: cpu_package_temp(&chips),
            utilization_pct,
            avg_freq_mhz,
            max_freq_mhz,
            power_w,
        },
        gpu: nvidia_gpu(),
        battery: battery(),
        // Fan RPM is not exposed via hwmon on this platform; the WMI/EC fan
        // interface must be verified before it can be reported.
        fans: Vec::new(),
    }
}

/// Current CPU package temperature without the sampling delay. Read-only.
pub fn cpu_temp_c() -> Option<f64> {
    cpu_package_temp(&hwmon::read())
}

/// Current AC/mains online state, if a Mains supply is present. Read-only.
pub fn ac_online() -> Option<bool> {
    supply_of_type("Mains").and_then(|d| read_trim(d.join("online")).map(|v| v == "1"))
}

pub(crate) fn cpu_package_temp(chips: &[HwmonChip]) -> Option<f64> {
    let core = chips.iter().find(|c| c.name.as_deref() == Some("coretemp"))?;
    core.temps
        .iter()
        .find(|t| t.label.as_deref() == Some("Package id 0"))
        .and_then(|t| t.celsius)
        .or_else(|| {
            core.temps
                .iter()
                .filter_map(|t| t.celsius)
                .max_by(f64::total_cmp)
        })
}

fn cpu_freqs() -> (Option<f64>, Option<f64>) {
    let mut sum = 0.0;
    let mut count = 0u32;
    let mut max = 0.0f64;
    if let Ok(rd) = fs::read_dir("/sys/devices/system/cpu") {
        for entry in rd.filter_map(|e| e.ok()) {
            if let Some(khz) = read_u64(entry.path().join("cpufreq/scaling_cur_freq")) {
                let mhz = khz as f64 / 1000.0;
                sum += mhz;
                count += 1;
                max = max.max(mhz);
            }
        }
    }
    if count == 0 {
        (None, None)
    } else {
        (Some(sum / count as f64), Some(max))
    }
}

fn read_rapl_energy_uj() -> Option<u64> {
    read_u64("/sys/class/powercap/intel-rapl:0/energy_uj")
}

fn nvidia_gpu() -> Option<GpuTelemetry> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,temperature.gpu,utilization.gpu,clocks.gr,power.draw",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let fields: Vec<&str> = stdout.lines().next()?.split(',').map(str::trim).collect();
    if fields.len() < 5 {
        return None;
    }
    Some(GpuTelemetry {
        name: fields[0].to_owned(),
        temp_c: fields[1].parse().ok(),
        utilization_pct: fields[2].parse().ok(),
        clock_mhz: fields[3].parse().ok(),
        power_w: fields[4].parse().ok(),
    })
}

fn battery() -> Option<BatteryTelemetry> {
    let dir = supply_of_type("Battery")?;
    let power_w = read_u64(dir.join("power_now"))
        .map(|uw| uw as f64 / 1e6)
        .or_else(|| {
            let ua = read_i64(dir.join("current_now"))?;
            let uv = read_u64(dir.join("voltage_now"))?;
            Some((ua.unsigned_abs() as f64 / 1e6) * (uv as f64 / 1e6))
        });
    Some(BatteryTelemetry {
        capacity_pct: read_u64(dir.join("capacity")),
        status: read_trim(dir.join("status")),
        power_w,
        ac_online: supply_of_type("Mains").and_then(|d| read_trim(d.join("online")).map(|v| v == "1")),
    })
}

fn supply_of_type(kind: &str) -> Option<PathBuf> {
    fs::read_dir("/sys/class/power_supply")
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| read_trim(p.join("type")).as_deref() == Some(kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hwmon::{HwmonChip, TempChannel};

    fn coretemp(temps: &[(u32, &str, f64)]) -> HwmonChip {
        HwmonChip {
            name: Some("coretemp".into()),
            path: String::new(),
            temps: temps
                .iter()
                .map(|(i, l, c)| TempChannel {
                    index: *i,
                    label: Some((*l).into()),
                    celsius: Some(*c),
                })
                .collect(),
            fans: Vec::new(),
            pwms: Vec::new(),
            powers: Vec::new(),
        }
    }

    #[test]
    fn utilization_half_busy() {
        let a = CpuTimes { idle: 100, total: 200 };
        let b = CpuTimes { idle: 150, total: 300 };
        assert_eq!(a.utilization(&b), Some(50.0));
    }

    #[test]
    fn utilization_rejects_zero_and_backwards() {
        let a = CpuTimes { idle: 10, total: 20 };
        assert_eq!(a.utilization(&a), None);
        let older = CpuTimes { idle: 5, total: 30 };
        assert_eq!(a.utilization(&older), None);
    }

    #[test]
    fn package_temp_prefers_package_label() {
        let chips = [coretemp(&[(1, "Package id 0", 63.0), (2, "Core 0", 51.0)])];
        assert_eq!(cpu_package_temp(&chips), Some(63.0));
    }

    #[test]
    fn package_temp_falls_back_to_hottest_core() {
        let chips = [coretemp(&[(2, "Core 0", 51.0), (3, "Core 1", 70.0)])];
        assert_eq!(cpu_package_temp(&chips), Some(70.0));
    }

    #[test]
    fn package_temp_none_without_coretemp() {
        let chips = [HwmonChip {
            name: Some("acpitz".into()),
            path: String::new(),
            temps: vec![TempChannel { index: 1, label: None, celsius: Some(40.0) }],
            fans: Vec::new(),
            pwms: Vec::new(),
            powers: Vec::new(),
        }];
        assert_eq!(cpu_package_temp(&chips), None);
    }
}
