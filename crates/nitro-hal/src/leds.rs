use crate::sysfs::{read_trim, read_u64};
use serde::Serialize;
use std::fs;

#[derive(Debug, Serialize)]
pub struct LedDevice {
    pub name: String,
    pub brightness: Option<u64>,
    pub max_brightness: Option<u64>,
    pub multi_index: Option<String>,
    pub multi_intensity: Option<String>,
    pub is_keyboard_candidate: bool,
}

pub fn read() -> Vec<LedDevice> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/class/leds") {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for p in dirs {
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let lname = name.to_ascii_lowercase();
            let is_keyboard_candidate =
                lname.contains("kbd") || lname.contains("keyboard") || lname.contains("rgb");
            out.push(LedDevice {
                brightness: read_u64(p.join("brightness")),
                max_brightness: read_u64(p.join("max_brightness")),
                multi_index: read_trim(p.join("multi_index")),
                multi_intensity: read_trim(p.join("multi_intensity")),
                is_keyboard_candidate,
                name,
            });
        }
    }
    out
}
