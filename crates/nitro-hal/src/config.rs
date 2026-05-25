//! Persistent daemon configuration at /etc/nitro/config.json, shared by the
//! daemon (reader) and the CLI (writer). JSON keeps the dependency surface to
//! serde_json, which is already used elsewhere. A missing or invalid file falls
//! back to safe defaults rather than failing.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

pub const CONFIG_PATH: &str = "/etc/nitro/config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Desired profile name when `auto_switch` is false.
    pub profile: String,
    /// Automatically pick a profile from AC/thermal state.
    pub auto_switch: bool,
    /// Desired battery charge cap (percent), if the hardware supports it.
    pub charge_limit: Option<u8>,
    /// CPU package temperature (°C) that forces the emergency Quiet profile.
    pub thermal_guard_c: f64,
    /// Evdev node the daemon listens on for the NitroSense/hotkey button.
    pub hotkey_device: Option<String>,
    /// Keycode emitted by that button (learned via `nitro learn-key`).
    pub hotkey_code: Option<u16>,
    /// Daemon control-loop period in seconds.
    pub poll_secs: u64,
    /// Persisted keyboard RGB state, re-applied by the daemon on boot/resume.
    pub rgb: Option<crate::rgb::RgbState>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            profile: "balanced".into(),
            auto_switch: false,
            charge_limit: None,
            thermal_guard_c: 95.0,
            hotkey_device: None,
            hotkey_code: None,
            poll_secs: 5,
            rgb: None,
        }
    }
}

impl Config {
    /// Load config, returning defaults if the file is absent or unparseable.
    pub fn load() -> Config {
        match fs::read_to_string(CONFIG_PATH) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    /// Persist config (creates /etc/nitro). Requires root.
    pub fn save(&self) -> io::Result<()> {
        if let Some(dir) = Path::new(CONFIG_PATH).parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        fs::write(CONFIG_PATH, json)
    }

    /// Sanitised poll period (never zero, capped to a sane maximum).
    pub fn poll_period(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.poll_secs.clamp(1, 60))
    }
}
