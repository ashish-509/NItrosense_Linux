//! Four-zone keyboard RGB via the Acer gaming-WMI keyboard attributes.
//!
//! per_zone_mode: "RRGGBB,RRGGBB,RRGGBB,RRGGBB,BRIGHTNESS" (static per-zone).
//! four_zone_mode: "mode,speed,brightness,direction,red,green,blue" (effects),
//! mode 0-7, speed 0-9, brightness 0-100, direction 1-2, rgb 0-255.
//! Capability-gated on the linuwu-sense keyboard directory being present.

use crate::acer;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const OFF: Rgb = Rgb { r: 0, g: 0, b: 0 };

    pub fn hex(self) -> String {
        format!("{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Parse a "RRGGBB" (optionally "#RRGGBB") hex colour.
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(Rgb {
            r: u8::from_str_radix(&s[0..2], 16).ok()?,
            g: u8::from_str_radix(&s[2..4], 16).ok()?,
            b: u8::from_str_radix(&s[4..6], 16).ok()?,
        })
    }
}

/// Named four-zone effects and their `four_zone_mode` id. The linuwu-sense
/// driver validates and accepts modes 0-7 (names taken from its documentation).
pub const EFFECTS: &[(&str, u8)] = &[
    ("static", 0),
    ("breathing", 1),
    ("neon", 2),
    ("wave", 3),
    ("shifting", 4),
    ("zoom", 5),
    ("meteor", 6),
    ("twinkling", 7),
];

/// Resolve an effect name (case-insensitive) or a literal "0".."7" to a mode id.
pub fn effect_mode(name: &str) -> Option<u8> {
    let n = name.trim().to_ascii_lowercase();
    EFFECTS
        .iter()
        .find(|(k, _)| *k == n)
        .map(|(_, v)| *v)
        .or_else(|| n.parse::<u8>().ok().filter(|m| *m <= 7))
}

/// Human-readable name for a mode id (falls back to "custom").
pub fn effect_name(mode: u8) -> &'static str {
    EFFECTS
        .iter()
        .find(|(_, v)| *v == mode)
        .map(|(k, _)| *k)
        .unwrap_or("custom")
}

/// The supported effect names, for help text.
pub fn effect_names() -> Vec<&'static str> {
    EFFECTS.iter().map(|(k, _)| *k).collect()
}

pub fn supported() -> bool {
    acer::kbd_dir().is_some()
}

pub fn get() -> Option<String> {
    acer::read_attr(&acer::kbd_dir()?, "per_zone_mode")
}

/// Read the raw `four_zone_mode` attribute ("mode,speed,brightness,dir,r,g,b").
/// This reflects the global backlight/effect state (WMI GET method 21).
pub fn get_four_zone() -> Option<String> {
    acer::read_attr(&acer::kbd_dir()?, "four_zone_mode")
}

/// Set an explicit colour for each of the four zones plus overall brightness via
/// `per_zone_mode`. Note: on the AN515-5x firmware the underlying static path is
/// broken (the keyboard goes dark); animated effects via [`set_effect`] do work.
pub fn set_zones(zones: [Rgb; 4], brightness: u8) -> io::Result<()> {
    let dir = acer::kbd_dir().ok_or_else(unsupported)?;
    let b = brightness.min(100);
    let value = format!(
        "{},{},{},{},{}",
        zones[0].hex(),
        zones[1].hex(),
        zones[2].hex(),
        zones[3].hex(),
        b
    );
    acer::write_attr(&dir, "per_zone_mode", &value)
}

/// Set all four zones to the same colour.
pub fn set_all(color: Rgb, brightness: u8) -> io::Result<()> {
    set_zones([color; 4], brightness)
}

pub fn off() -> io::Result<()> {
    set_all(Rgb::OFF, 0)
}

/// Drive an animated effect via four_zone_mode.
pub fn set_effect(mode: u8, speed: u8, brightness: u8, direction: u8, color: Rgb) -> io::Result<()> {
    let dir = acer::kbd_dir().ok_or_else(unsupported)?;
    let value = format!(
        "{},{},{},{},{},{},{}",
        mode.min(7),
        speed.min(9),
        brightness.min(100),
        direction.clamp(1, 2),
        color.r,
        color.g,
        color.b,
    );
    acer::write_attr(&dir, "four_zone_mode", &value)
}

/// Parse the raw `per_zone_mode` string ("hex,hex,hex,hex,brightness") into the
/// four zone colours and the brightness value.
pub fn parse_zone_string(s: &str) -> Option<([Rgb; 4], u8)> {
    let parts: Vec<&str> = s.trim().split(',').collect();
    if parts.len() < 5 {
        return None;
    }
    let mut zones = [Rgb::OFF; 4];
    for (i, z) in zones.iter_mut().enumerate() {
        *z = Rgb::parse(parts[i])?;
    }
    let brightness = parts[4].trim().parse::<u8>().ok()?;
    Some((zones, brightness))
}

/// A persisted keyboard-RGB configuration the daemon can re-apply on boot or
/// after resume, so a chosen colour survives a reboot instead of reverting to
/// the firmware default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RgbState {
    /// All zones off.
    Off,
    /// Static per-zone colours (hex "RRGGBB") plus overall brightness.
    Zones { colors: [String; 4], brightness: u8 },
    /// An animated effect.
    Effect {
        mode: u8,
        speed: u8,
        brightness: u8,
        direction: u8,
        color: String,
    },
}

impl RgbState {
    /// Apply this state to the hardware.
    pub fn apply(&self) -> io::Result<()> {
        match self {
            RgbState::Off => off(),
            RgbState::Zones { colors, brightness } => {
                let mut zones = [Rgb::OFF; 4];
                for (i, z) in zones.iter_mut().enumerate() {
                    *z = Rgb::parse(&colors[i]).ok_or_else(invalid_color)?;
                }
                set_zones(zones, *brightness)
            }
            RgbState::Effect {
                mode,
                speed,
                brightness,
                direction,
                color,
            } => {
                let c = Rgb::parse(color).ok_or_else(invalid_color)?;
                set_effect(*mode, *speed, *brightness, *direction, c)
            }
        }
    }

    /// Return a copy with the brightness replaced (colours/effect preserved).
    pub fn with_brightness(&self, brightness: u8) -> RgbState {
        let b = brightness.min(100);
        match self {
            RgbState::Off => RgbState::Off,
            RgbState::Zones { colors, .. } => RgbState::Zones {
                colors: colors.clone(),
                brightness: b,
            },
            RgbState::Effect {
                mode,
                speed,
                direction,
                color,
                ..
            } => RgbState::Effect {
                mode: *mode,
                speed: *speed,
                brightness: b,
                direction: *direction,
                color: color.clone(),
            },
        }
    }
}

fn invalid_color() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid RGB colour value")
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "keyboard RGB unavailable (linuwu-sense module not loaded)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_hex_round_trip() {
        let c = Rgb::parse("4287f5").unwrap();
        assert_eq!(c, Rgb { r: 0x42, g: 0x87, b: 0xf5 });
        assert_eq!(c.hex(), "4287f5");
        assert_eq!(Rgb::parse("#ff0000"), Some(Rgb { r: 255, g: 0, b: 0 }));
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(Rgb::parse("xyz").is_none());
        assert!(Rgb::parse("12345").is_none());
        assert!(Rgb::parse("gggggg").is_none());
    }

    #[test]
    fn effect_mode_resolves_names_and_numbers() {
        assert_eq!(effect_mode("breathing"), Some(1));
        assert_eq!(effect_mode("WAVE"), Some(3));
        assert_eq!(effect_mode("7"), Some(7));
        assert_eq!(effect_mode("8"), None);
        assert_eq!(effect_mode("nope"), None);
        assert_eq!(effect_name(6), "meteor");
    }

    #[test]
    fn zone_string_parses() {
        let (zones, b) = parse_zone_string("ff0000,00ff00,0000ff,ffffff,80").unwrap();
        assert_eq!(zones[0], Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(zones[3], Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(b, 80);
        assert!(parse_zone_string("bad").is_none());
    }

    #[test]
    fn rgb_state_brightness_and_serde_round_trip() {
        let s = RgbState::Zones {
            colors: [
                "ff0000".into(),
                "00ff00".into(),
                "0000ff".into(),
                "ffffff".into(),
            ],
            brightness: 100,
        };
        let dimmed = s.with_brightness(150);
        match dimmed {
            RgbState::Zones { brightness, .. } => assert_eq!(brightness, 100),
            _ => panic!("expected zones"),
        }
        let json = serde_json::to_string(&s).unwrap();
        let back: RgbState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
