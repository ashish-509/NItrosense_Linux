//! Performance/thermal profiles expressed purely through the verified
//! intel_pstate/cpufreq levers (governor + EPP + turbo). No fan or firmware
//! thermal-table control is involved, because those interfaces are not yet
//! verified on this hardware. Every profile therefore only changes CPU behaviour.

use crate::{control, fan, platform_profile};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Coolest/quietest achievable via CPU levers alone: powersave + power EPP,
    /// turbo disabled. Also the emergency thermal-guard target.
    Quiet,
    /// Default everyday balance.
    Balanced,
    /// Biased toward performance but still scales down when idle.
    Performance,
    /// Pinned to the performance governor with turbo for maximum sustained clocks.
    Turbo,
}

impl Profile {
    pub const ALL: [Profile; 4] = [
        Profile::Quiet,
        Profile::Balanced,
        Profile::Performance,
        Profile::Turbo,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Profile::Quiet => "quiet",
            Profile::Balanced => "balanced",
            Profile::Performance => "performance",
            Profile::Turbo => "turbo",
        }
    }

    pub fn parse(s: &str) -> Option<Profile> {
        match s.trim().to_ascii_lowercase().as_str() {
            "quiet" | "q" => Some(Profile::Quiet),
            "balanced" | "balance" | "b" => Some(Profile::Balanced),
            "performance" | "perf" | "p" => Some(Profile::Performance),
            "turbo" | "max" | "t" => Some(Profile::Turbo),
            _ => None,
        }
    }

    /// The next profile when cycling with the NitroSense key.
    pub fn next(self) -> Profile {
        match self {
            Profile::Quiet => Profile::Balanced,
            Profile::Balanced => Profile::Performance,
            Profile::Performance => Profile::Turbo,
            Profile::Turbo => Profile::Quiet,
        }
    }

    /// (governor, energy_performance_preference, turbo_enabled). Pure — the whole
    /// mapping is data so it can be unit tested without touching hardware.
    fn levers(self) -> (&'static str, Option<&'static str>, bool) {
        match self {
            Profile::Quiet => ("powersave", Some("power"), false),
            Profile::Balanced => ("powersave", Some("balance_performance"), true),
            Profile::Performance => ("powersave", Some("performance"), true),
            Profile::Turbo => ("performance", None, true),
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Profile::Quiet => "quiet: powersave governor, power EPP, turbo off",
            Profile::Balanced => "balanced: powersave governor, balanced EPP, turbo on",
            Profile::Performance => "performance: powersave governor, performance EPP, turbo on",
            Profile::Turbo => "turbo: performance governor, turbo on",
        }
    }

    /// Preferred ACPI platform_profile names, best first. The first one the
    /// firmware advertises is used.
    fn platform_prefs(self) -> &'static [&'static str] {
        match self {
            Profile::Quiet => &["quiet", "low-power", "cool"],
            Profile::Balanced => &["balanced"],
            Profile::Performance => &["performance", "balanced-performance"],
            Profile::Turbo => &["turbo", "performance"],
        }
    }

    /// Fan policy for this profile (only used when the WMI fan interface exists).
    fn fan_mode(self) -> fan::FanMode {
        match self {
            Profile::Turbo => fan::FanMode::Max,
            _ => fan::FanMode::Auto,
        }
    }
}

/// Apply a profile. Layers, from most hardware-specific to always-available:
///   1. firmware ACPI platform_profile (if exposed),
///   2. Acer WMI fan policy (if the linuwu-sense module is loaded),
///   3. the verified intel_pstate/cpufreq levers (always).
/// Layers 1 and 2 are best-effort and capability-gated so a machine without them
/// still gets correct CPU behaviour. Requires root.
pub fn apply(profile: Profile) -> io::Result<()> {
    if platform_profile::supported() {
        if let Some(name) = platform_profile::pick(profile.platform_prefs()) {
            let _ = platform_profile::set(&name);
        }
    }
    if fan::supported() {
        let _ = fan::set(profile.fan_mode());
    }
    let (governor, epp, turbo) = profile.levers();
    control::apply(governor, epp, turbo)
}

/// Decide which profile the daemon should hold given power and thermal state.
/// Pure so it is fully unit tested. The thermal guard always wins and forces
/// Quiet, the safest CPU state, because we cannot raise fan speed.
pub fn auto_profile(ac_online: Option<bool>, temp_c: Option<f64>, guard_c: f64) -> Profile {
    if let Some(t) = temp_c {
        if t >= guard_c {
            return Profile::Quiet;
        }
    }
    match ac_online {
        Some(true) => Profile::Performance,
        _ => Profile::Balanced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips() {
        for p in Profile::ALL {
            assert_eq!(Profile::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn parse_rejects_unknown() {
        assert_eq!(Profile::parse("ludicrous"), None);
    }

    #[test]
    fn cycle_visits_all_four() {
        let mut p = Profile::Quiet;
        let mut seen = vec![p];
        for _ in 0..3 {
            p = p.next();
            seen.push(p);
        }
        assert_eq!(seen, Profile::ALL.to_vec());
        assert_eq!(Profile::Turbo.next(), Profile::Quiet);
    }

    #[test]
    fn thermal_guard_forces_quiet() {
        assert_eq!(auto_profile(Some(true), Some(96.0), 95.0), Profile::Quiet);
        assert_eq!(auto_profile(Some(false), Some(99.9), 95.0), Profile::Quiet);
    }

    #[test]
    fn auto_uses_ac_state_when_cool() {
        assert_eq!(auto_profile(Some(true), Some(60.0), 95.0), Profile::Performance);
        assert_eq!(auto_profile(Some(false), Some(60.0), 95.0), Profile::Balanced);
        assert_eq!(auto_profile(None, None, 95.0), Profile::Balanced);
    }

    #[test]
    fn quiet_disables_turbo() {
        assert_eq!(Profile::Quiet.levers().2, false);
        assert_eq!(Profile::Turbo.levers().2, true);
    }
}
