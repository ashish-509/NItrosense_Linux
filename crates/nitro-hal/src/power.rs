use crate::sysfs::{read_trim, read_u64};
use serde::Serialize;
use std::fs;

#[derive(Debug, Serialize)]
pub struct PowerSupply {
    pub name: String,
    pub kind: Option<String>,
    pub online: Option<bool>,
    pub status: Option<String>,
    pub capacity: Option<u64>,
    pub charge_control_start_threshold: Option<u64>,
    pub charge_control_end_threshold: Option<u64>,
    pub charge_limit_supported: bool,
    pub health: Option<String>,
}

pub fn read() -> Vec<PowerSupply> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/power_supply") {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for p in dirs {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            out.push(PowerSupply {
                name,
                kind: read_trim(p.join("type")),
                online: read_trim(p.join("online")).map(|v| v == "1"),
                status: read_trim(p.join("status")),
                capacity: read_u64(p.join("capacity")),
                charge_control_start_threshold: read_u64(p.join("charge_control_start_threshold")),
                charge_control_end_threshold: read_u64(p.join("charge_control_end_threshold")),
                charge_limit_supported: p.join("charge_control_end_threshold").exists(),
                health: read_trim(p.join("health")),
            });
        }
    }
    out
}
