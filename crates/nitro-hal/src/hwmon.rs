use crate::sysfs::{read_i64, read_trim, read_u64};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct TempChannel {
    pub index: u32,
    pub label: Option<String>,
    pub celsius: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct FanChannel {
    pub index: u32,
    pub label: Option<String>,
    pub rpm: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct PwmChannel {
    pub index: u32,
    pub raw: Option<u64>,
    pub enable: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct PowerChannel {
    pub index: u32,
    pub label: Option<String>,
    pub watts: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct HwmonChip {
    pub name: Option<String>,
    pub path: String,
    pub temps: Vec<TempChannel>,
    pub fans: Vec<FanChannel>,
    pub pwms: Vec<PwmChannel>,
    pub powers: Vec<PowerChannel>,
}

pub fn read() -> Vec<HwmonChip> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/hwmon") {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            out.push(collect(&dir));
        }
    }
    out
}

fn collect(path: &Path) -> HwmonChip {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    let mut pwms = Vec::new();
    let mut powers = Vec::new();

    if let Ok(rd) = fs::read_dir(path) {
        let names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        for name in &names {
            if let Some(idx) = channel_index(name, "temp", "_input") {
                temps.push(TempChannel {
                    index: idx,
                    label: read_trim(path.join(format!("temp{idx}_label"))),
                    celsius: read_i64(path.join(name)).map(|v| v as f64 / 1000.0),
                });
            } else if let Some(idx) = channel_index(name, "fan", "_input") {
                fans.push(FanChannel {
                    index: idx,
                    label: read_trim(path.join(format!("fan{idx}_label"))),
                    rpm: read_u64(path.join(name)),
                });
            } else if let Some(idx) = channel_index(name, "power", "_input") {
                powers.push(PowerChannel {
                    index: idx,
                    label: read_trim(path.join(format!("power{idx}_label"))),
                    // hwmon reports power in microwatts
                    watts: read_u64(path.join(name)).map(|v| v as f64 / 1_000_000.0),
                });
            } else if let Some(idx) = name
                .strip_prefix("pwm")
                .and_then(|rest| rest.parse::<u32>().ok())
            {
                pwms.push(PwmChannel {
                    index: idx,
                    raw: read_u64(path.join(name)),
                    enable: read_u64(path.join(format!("pwm{idx}_enable"))),
                });
            }
        }
    }

    temps.sort_by_key(|c| c.index);
    fans.sort_by_key(|c| c.index);
    pwms.sort_by_key(|c| c.index);
    powers.sort_by_key(|c| c.index);

    HwmonChip {
        name: read_trim(path.join("name")),
        path: path.display().to_string(),
        temps,
        fans,
        pwms,
        powers,
    }
}

fn channel_index(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    name.strip_prefix(prefix)?.strip_suffix(suffix)?.parse().ok()
}
