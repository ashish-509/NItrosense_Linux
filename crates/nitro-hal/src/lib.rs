//! Hardware abstraction layer for Acer Nitro laptops.
//!
//! The discovery/telemetry modules (dmi, ec, hwmon, input, leds, pci, power,
//! probe, telemetry, wmi) are strictly read-only. Write paths are confined to a
//! few clearly marked modules — `control` (intel_pstate/cpufreq), `profile`
//! (profiles built on the layers below), `platform_profile` (ACPI firmware
//! profiles), `fan` and `rgb` (Acer gaming-WMI via the linuwu-sense module) and
//! `battery` (charge limit) — each of which validates or capability-gates every
//! value before writing. The `acer` module only discovers the WMI sysfs surface.
//! `config` and `state` persist daemon settings/status as JSON. No
//! embedded-controller or WMI register is ever written from a guessed offset;
//! the WMI writes go through firmware-validated, documented attribute files.

mod sysfs;

pub mod acer;
pub mod battery;
pub mod config;
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
pub mod state;
pub mod telemetry;
pub mod wmi;

pub use config::Config;
pub use control::{CpuBackup, CpuStatus};
pub use probe::{run, CapabilitySummary, Cpu, Kernel, Report};
pub use profile::Profile;
pub use state::State;
pub use telemetry::Telemetry;
