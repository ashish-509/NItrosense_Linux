//! Hardware abstraction layer for Acer Nitro laptops.

mod sysfs;

pub mod acer;
pub mod battery;
pub mod control;
pub mod dmi;
pub mod ec;
pub mod evdev;
pub mod fan;
pub mod hwmon;
pub mod input;
pub mod leds;
pub mod pci;
pub mod platform_profile;
pub mod power;
pub mod probe;
pub mod profile;
pub mod rgb;
pub mod telemetry;
pub mod wmi;

pub use probe::{run, CapabilitySummary, Cpu, Kernel, Report};
pub use control::{CpuBackup, CpuStatus};
pub use profile::Profile;
