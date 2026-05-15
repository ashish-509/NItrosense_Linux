use crate::sysfs::read_trim;
use serde::Serialize;
use std::fs;

#[derive(Debug, Serialize)]
pub struct PciDevice {
    pub slot: String,
    pub class: Option<String>,
    pub vendor: Option<String>,
    pub device: Option<String>,
    pub vendor_name: Option<&'static str>,
    pub driver: Option<String>,
}

pub fn read_display() -> Vec<PciDevice> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir("/sys/bus/pci/devices") {
        let mut slots: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        slots.sort();
        for p in slots {
            let class = read_trim(p.join("class"));
            // PCI base class 0x03 == display controller (VGA / 3D / other display).
            if !class.as_deref().map_or(false, |c| c.starts_with("0x03")) {
                continue;
            }
            let vendor = read_trim(p.join("vendor"));
            let vendor_name = match vendor.as_deref() {
                Some("0x10de") => Some("NVIDIA"),
                Some("0x8086") => Some("Intel"),
                Some("0x1002") => Some("AMD"),
                _ => None,
            };
            let driver = fs::read_link(p.join("driver"))
                .ok()
                .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
            out.push(PciDevice {
                slot: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                class,
                vendor,
                device: read_trim(p.join("device")),
                vendor_name,
                driver,
            });
        }
    }
    out
}
