//! Battery charge-limit control via the standard power_supply sysfs interface
//! (`charge_control_end_threshold`). This is capability-gated: if the running
//! kernel/firmware does not expose the threshold file, every write is refused
//! rather than guessed. On the AN515-56 the mainline kernel does not currently
//! expose this file, so `supported()` returns false until an acer-wmi-battery
//! style interface is confirmed — no register is ever guessed.

use crate::acer;
use crate::sysfs::read_u64;
use std::fs;
use std::io;
use std::path::PathBuf;

const PS_BASE: &str = "/sys/class/power_supply";
const END: &str = "charge_control_end_threshold";
const START: &str = "charge_control_start_threshold";

/// Minimum accepted end threshold. Setting the cap too low can prevent the pack
/// from charging at all, so we clamp to a conservative safe floor.
pub const MIN_LIMIT: u8 = 20;
pub const MAX_LIMIT: u8 = 100;

/// The fixed cap the Acer `battery_limiter` toggle enforces when enabled; the
/// firmware exposes no other value on that interface.
pub const FIXED_CAP: u8 = 80;

/// Which charge-limit interface the running kernel/firmware exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    /// Standard `charge_control_end_threshold`: any percent in range works.
    Threshold,
    /// Acer linuwu-sense `battery_limiter`: a fixed 80% cap on/off toggle only.
    FixedToggle,
}

fn battery_dir() -> Option<PathBuf> {
    fs::read_dir(PS_BASE)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            fs::read_to_string(p.join("type"))
                .map(|t| t.trim() == "Battery")
                .unwrap_or(false)
        })
}

/// Report which charge-limit interface exists, preferring the arbitrary-percent
/// sysfs threshold over the Acer fixed-cap toggle.
pub fn limit_kind() -> Option<LimitKind> {
    if battery_dir().map(|d| d.join(END).exists()).unwrap_or(false) {
        return Some(LimitKind::Threshold);
    }
    if acer::sense_dir()
        .map(|d| d.join("battery_limiter").exists())
        .unwrap_or(false)
    {
        return Some(LimitKind::FixedToggle);
    }
    None
}

/// True when any charge-limit interface exists.
pub fn supported() -> bool {
    limit_kind().is_some()
}

pub fn get_limit() -> Option<u8> {
    if let Some(dir) = battery_dir() {
        if let Some(v) = read_u64(dir.join(END)) {
            return Some(v.min(100) as u8);
        }
    }
    if let Some(sense) = acer::sense_dir() {
        if acer::read_attr(&sense, "battery_limiter").as_deref() == Some("1") {
            return Some(FIXED_CAP);
        }
    }
    None
}

/// Set the charge end threshold. Validates the range and refuses when no
/// interface exists instead of writing to an unknown location.
pub fn set_limit(percent: u8) -> io::Result<()> {
    if !(MIN_LIMIT..=MAX_LIMIT).contains(&percent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("charge limit must be {MIN_LIMIT}..={MAX_LIMIT}, got {percent}"),
        ));
    }
    // Preferred: the standard sysfs end threshold (accepts an arbitrary percent).
    if let Some(dir) = battery_dir() {
        let end = dir.join(END);
        if end.exists() {
            // Some firmwares require start < end; only touch start if writable and
            // it would otherwise exceed the new end value.
            let start = dir.join(START);
            if start.exists() {
                if let Some(cur_start) = read_u64(&start) {
                    if cur_start as u8 >= percent {
                        let _ = fs::write(&start, percent.saturating_sub(5).to_string());
                    }
                }
            }
            return fs::write(end, percent.to_string());
        }
    }
    // Fallback: the Acer gaming-WMI limiter, a fixed 80% cap toggle.
    if let Some(sense) = acer::sense_dir() {
        if sense.join("battery_limiter").exists() {
            let enable = if percent < 100 { "1" } else { "0" };
            return acer::write_attr(&sense, "battery_limiter", enable);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no battery charge-limit interface exposed by this kernel/firmware",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(set_limit(0).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            set_limit(101).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
