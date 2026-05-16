use crate::sysfs::{read_trim, read_u64};
use serde::Serialize;
use std::fs;
use std::path::Path;

pub const GAMING_LABEL: &str = "Acer gaming WMI (fan/perf/RGB methods)";

// GUIDs are stored uppercase; sysfs modaliases are matched case-insensitively.
const KNOWN: &[(&str, &str)] = &[
    ("7A4DDFE7-5B5D-40B4-8595-4408E0CC7F56", GAMING_LABEL),
    ("67C3371D-95A3-4C37-BB61-DD47B491DAAB", "Acer WMID"),
    ("431F16ED-0C2B-444C-B267-27DEB140CF9C", "Acer WMBB"),
];

#[derive(Debug, Serialize)]
pub struct WmiDevice {
    pub device: String,
    pub guid: Option<String>,
    pub modalias: Option<String>,
    pub object_id: Option<String>,
    pub instance_count: Option<u64>,
    pub expensive: Option<bool>,
    pub driver: Option<String>,
    pub known: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct AcerPlatform {
    pub path: String,
    pub attributes: Vec<(String, String)>,
}

pub fn read() -> Vec<WmiDevice> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/bus/wmi/devices") {
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for p in dirs {
            let device = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let modalias = read_trim(p.join("modalias"));
            let guid = modalias
                .as_deref()
                .and_then(|m| m.strip_prefix("wmi:"))
                .map(str::to_owned)
                .or_else(|| Some(device.clone()));
            let known = guid.as_deref().and_then(known_label);
            let driver = fs::read_link(p.join("driver"))
                .ok()
                .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
            out.push(WmiDevice {
                device,
                guid,
                modalias,
                object_id: read_trim(p.join("object_id")),
                instance_count: read_u64(p.join("instance_count")),
                expensive: read_trim(p.join("expensive")).map(|v| v == "1"),
                driver,
                known,
            });
        }
    }
    out
}

pub fn acer_platform() -> Option<AcerPlatform> {
    let path = Path::new("/sys/devices/platform/acer-wmi");
    if !path.exists() {
        return None;
    }
    let mut attributes = Vec::new();
    if let Ok(rd) = fs::read_dir(path) {
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for n in names {
            if matches!(n.as_str(), "uevent" | "modalias") {
                continue;
            }
            if let Some(v) = read_trim(path.join(&n)) {
                attributes.push((n, v));
            }
        }
    }
    Some(AcerPlatform {
        path: path.display().to_string(),
        attributes,
    })
}

fn known_label(guid: &str) -> Option<&'static str> {
    let up = guid.to_ascii_uppercase();
    KNOWN.iter().find(|(g, _)| *g == up).map(|(_, l)| *l)
}
