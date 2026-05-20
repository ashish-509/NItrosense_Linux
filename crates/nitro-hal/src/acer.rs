//! Discovery of the Acer gaming-WMI sysfs surface exposed by the linuwu-sense
//! kernel module. These are firmware-mediated WMI methods (the same ones the
//! official NitroSense/PredatorSense apps use), *not* raw EC register writes, so
//! the firmware validates every value. Everything here is capability-gated: if
//! the module is not loaded the attribute files are absent and callers treat the
//! feature as unsupported rather than writing anywhere blind.
//!
//! Documented paths (see the linuwu-sense README):
//!   Nitro:    /sys/module/linuwu_sense/drivers/platform:acer-wmi/acer-wmi/nitro_sense
//!   Predator: .../acer-wmi/predator_sense
//!   Keyboard: .../acer-wmi/four_zoned_kb

use crate::sysfs::read_trim;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MODULE_BASE: &str = "/sys/module/linuwu_sense/drivers/platform:acer-wmi/acer-wmi";
const PLATFORM_BASE: &str = "/sys/devices/platform/acer-wmi";

/// Directory holding fan_speed/battery_limiter/etc. (nitro_sense or predator_sense).
pub fn sense_dir() -> Option<PathBuf> {
    for base in [MODULE_BASE, PLATFORM_BASE] {
        for name in ["nitro_sense", "predator_sense"] {
            let dir = Path::new(base).join(name);
            if dir.join("fan_speed").exists() {
                return Some(dir);
            }
        }
    }
    None
}

/// Directory holding the four-zone keyboard RGB attributes.
pub fn kbd_dir() -> Option<PathBuf> {
    for base in [MODULE_BASE, PLATFORM_BASE] {
        let dir = Path::new(base).join("four_zoned_kb");
        if dir.join("per_zone_mode").exists() {
            return Some(dir);
        }
    }
    None
}

/// True when the linuwu-sense module appears to be loaded at all.
pub fn module_loaded() -> bool {
    Path::new(MODULE_BASE).exists() || sense_dir().is_some()
}

pub(crate) fn read_attr(dir: &Path, file: &str) -> Option<String> {
    read_trim(dir.join(file))
}

/// Write a sysfs attribute, refusing (never creating) an absent file.
pub(crate) fn write_attr(dir: &Path, file: &str, value: &str) -> io::Result<()> {
    let path = dir.join(file);
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{file} not available (is the linuwu-sense module loaded?)"),
        ));
    }
    fs::write(path, value)
}
