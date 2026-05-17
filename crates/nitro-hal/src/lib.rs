//! Hardware abstraction layer for Acer Nitro laptops.

mod sysfs;

pub mod dmi;
pub mod ec;
pub mod hwmon;
pub mod input;
pub mod leds;
pub mod pci;
pub mod power;
pub mod wmi;
