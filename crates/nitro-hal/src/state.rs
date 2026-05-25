//! Runtime state the daemon publishes to /run/nitro/state.json so the (non-root)
//! CLI can report what the daemon is doing without any IPC socket. World-readable
//! by design; it contains only status, never secrets.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const RUN_DIR: &str = "/run/nitro";
pub const STATE_PATH: &str = "/run/nitro/state.json";
pub const PID_PATH: &str = "/run/nitro/nitrod.pid";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub profile: String,
    pub auto_switch: bool,
    pub cpu_temp_c: Option<f64>,
    pub ac_online: Option<bool>,
    pub thermal_guard_active: bool,
    pub charge_limit: Option<u8>,
    pub hotkey_bound: bool,
    pub updated_unix: u64,
    pub daemon_pid: u32,
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn write(state: &State) -> io::Result<()> {
    fs::create_dir_all(RUN_DIR)?;
    let json =
        serde_json::to_string_pretty(state).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(STATE_PATH, json)
}

pub fn read() -> Option<State> {
    serde_json::from_str(&fs::read_to_string(STATE_PATH).ok()?).ok()
}

/// True if a daemon process matching the recorded PID is currently alive.
pub fn daemon_running() -> bool {
    let Some(state) = read() else {
        return false;
    };
    if state.daemon_pid == 0 {
        return false;
    }
    Path::new(&format!("/proc/{}", state.daemon_pid)).exists()
}
