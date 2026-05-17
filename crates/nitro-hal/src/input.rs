use serde::Serialize;
use std::fs;

#[derive(Debug, Serialize)]
pub struct InputDevice {
    pub name: Option<String>,
    pub phys: Option<String>,
    pub sysfs: Option<String>,
    pub handlers: Vec<String>,
    pub ev_keys: bool,
    pub hotkey_candidate: bool,
}

pub fn read() -> Vec<InputDevice> {
    let content = match fs::read_to_string("/proc/bus/input/devices") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .map(parse_block)
        .collect()
}

fn parse_block(block: &str) -> InputDevice {
    let mut name = None;
    let mut phys = None;
    let mut sysfs = None;
    let mut handlers = Vec::new();
    let mut ev_keys = false;

    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("N: Name=") {
            name = Some(rest.trim().trim_matches('"').to_owned());
        } else if let Some(rest) = line.strip_prefix("P: Phys=") {
            phys = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("S: Sysfs=") {
            sysfs = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("H: Handlers=") {
            handlers = rest.split_whitespace().map(str::to_owned).collect();
        } else if let Some(rest) = line.strip_prefix("B: EV=") {
            // EV bitmask; bit 1 (EV_KEY) indicates the device emits key events.
            if let Ok(bits) = u64::from_str_radix(rest.trim(), 16) {
                ev_keys = bits & (1 << 1) != 0;
            }
        }
    }

    let hotkey_candidate = name.as_deref().map_or(false, |n| {
        let l = n.to_ascii_lowercase();
        l.contains("wmi")
            || l.contains("acer")
            || l.contains("hotkey")
            || l.contains("video bus")
            || l.contains("at translated set 2 keyboard")
    });

    InputDevice {
        name,
        phys,
        sysfs,
        handlers,
        ev_keys,
        hotkey_candidate,
    }
}
