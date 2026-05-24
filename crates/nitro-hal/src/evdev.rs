//! Minimal evdev reader for the NitroSense/hotkey button, implemented directly
//! against the kernel `input_event` binary layout so it pulls in no extra crates.
//! It is read-only: it never writes to input devices. The daemon uses it to react
//! to the hotkey; `nitro learn-key` uses it to capture the (unknown) keycode
//! interactively rather than guessing one.

use crate::input;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// `struct input_event` on 64-bit Linux: two 64-bit timeval longs (16 bytes),
/// then u16 type, u16 code, i32 value.
const EVENT_SIZE: usize = 24;
const EV_KEY: u16 = 0x01;
const KEY_PRESS: i32 = 1;

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

fn parse(buf: &[u8; EVENT_SIZE]) -> InputEvent {
    InputEvent {
        kind: u16::from_ne_bytes([buf[16], buf[17]]),
        code: u16::from_ne_bytes([buf[18], buf[19]]),
        value: i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]),
    }
}

fn read_event(f: &mut File) -> io::Result<InputEvent> {
    let mut buf = [0u8; EVENT_SIZE];
    f.read_exact(&mut buf)?;
    Ok(parse(&buf))
}

/// Block until the first key *press* on `dev` and return its keycode.
pub fn wait_for_keypress(dev: &Path) -> io::Result<u16> {
    let mut f = File::open(dev)?;
    loop {
        let ev = read_event(&mut f)?;
        if ev.kind == EV_KEY && ev.value == KEY_PRESS {
            return Ok(ev.code);
        }
    }
}

/// Watch `dev` forever, invoking `on_press(code)` for every key press. The
/// closure returns `false` to stop watching.
pub fn watch_keys<F: FnMut(u16) -> bool>(dev: &Path, mut on_press: F) -> io::Result<()> {
    let mut f = File::open(dev)?;
    loop {
        let ev = read_event(&mut f)?;
        if ev.kind == EV_KEY && ev.value == KEY_PRESS && !on_press(ev.code) {
            return Ok(());
        }
    }
}

/// Best-effort resolution of the Acer hotkey evdev node from
/// /proc/bus/input/devices, preferring the "Acer WMI hotkeys" device.
pub fn find_hotkey_device() -> Option<PathBuf> {
    let devices = input::read();
    let pick = devices
        .iter()
        .find(|d| {
            d.name
                .as_deref()
                .is_some_and(|n| n.to_ascii_lowercase().contains("wmi hotkeys"))
        })
        .or_else(|| {
            devices
                .iter()
                .find(|d| d.hotkey_candidate && event_node(d).is_some())
        })?;
    event_node(pick)
}

fn event_node(dev: &input::InputDevice) -> Option<PathBuf> {
    dev.handlers
        .iter()
        .find(|h| h.starts_with("event"))
        .map(|h| PathBuf::from(format!("/dev/input/{h}")))
}
