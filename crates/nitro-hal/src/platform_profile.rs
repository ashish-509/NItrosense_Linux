//! Firmware thermal profiles via the standard ACPI platform_profile interface.
//! This is a stable kernel ABI (no third-party module needed where the firmware
//! or acer-wmi exposes it) and every write is validated against the advertised
//! choices, so an unknown value is never written.

use crate::sysfs::read_trim;
use std::fs;
use std::io;
use std::path::Path;

const PROFILE: &str = "/sys/firmware/acpi/platform_profile";
const CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";

pub fn supported() -> bool {
    Path::new(PROFILE).exists()
}

pub fn choices() -> Vec<String> {
    read_trim(CHOICES)
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

pub fn get() -> Option<String> {
    read_trim(PROFILE)
}

pub fn set(name: &str) -> io::Result<()> {
    let choices = choices();
    if !choices.iter().any(|c| c == name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("profile '{name}' not available; options: {}", choices.join(", ")),
        ));
    }
    fs::write(PROFILE, name)
}

/// Return the first preference that the firmware actually advertises.
pub fn pick(prefs: &[&str]) -> Option<String> {
    let choices = choices();
    prefs.iter().find_map(|p| {
        choices
            .iter()
            .find(|c| c.eq_ignore_ascii_case(p))
            .cloned()
    })
}
