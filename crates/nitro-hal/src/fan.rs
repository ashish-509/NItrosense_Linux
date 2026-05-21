//! CPU/GPU fan control via the Acer gaming-WMI `fan_speed` attribute.
//!
//! Format (per linuwu-sense): "cpu,gpu" where each value is 0 (auto), 1 (minimum,
//! not recommended) or 2..=100 (percent). The firmware validates the request, and
//! the whole surface is capability-gated on the module being loaded.

use crate::acer;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FanMode {
    /// Return both fans to firmware automatic control.
    Auto,
    /// Both fans to 100%.
    Max,
    /// Explicit CPU and GPU duty percentages (clamped to 0..=100).
    Manual { cpu: u8, gpu: u8 },
}

pub fn supported() -> bool {
    acer::sense_dir().is_some()
}

/// Raw "cpu,gpu" string as reported by the driver, if available.
pub fn get() -> Option<String> {
    acer::read_attr(&acer::sense_dir()?, "fan_speed")
}

pub fn set(mode: FanMode) -> io::Result<()> {
    let dir = acer::sense_dir().ok_or_else(unsupported)?;
    let (cpu, gpu) = match mode {
        FanMode::Auto => (0, 0),
        FanMode::Max => (100, 100),
        FanMode::Manual { cpu, gpu } => (cpu.min(100), gpu.min(100)),
    };
    acer::write_attr(&dir, "fan_speed", &format!("{cpu},{gpu}"))
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "fan control unavailable (linuwu-sense module not loaded)",
    )
}
