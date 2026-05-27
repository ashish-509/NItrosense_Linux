//! nitrod — the privileged background daemon.
//!
//! Responsibilities (all built on verified levers only):
//!   * hold the configured performance/thermal profile (cpufreq),
//!   * optionally auto-switch profile from AC/thermal state,
//!   * enforce an emergency thermal guard (force Quiet above a threshold),
//!   * apply the battery charge limit when the kernel exposes it,
//!   * react to the NitroSense/hotkey button (cycle profile) once learned.
//!
//! It reloads /etc/nitro/config.json every loop iteration (so the CLI just edits
//! that file) and publishes status to /run/nitro/state.json. On stop/crash,
//! systemd runs `nitrod --restore`, which returns the CPU to balanced defaults —
//! fans are never touched by us, so they stay under firmware control throughout.

use nitro_hal::config::Config;
use nitro_hal::profile::{self, Profile};
use nitro_hal::state::{self, State};
use nitro_hal::{battery, control, evdev, fan, platform_profile, rgb, telemetry};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;

fn main() -> ExitCode {
    if std::env::args().skip(1).any(|a| a == "--restore") {
        return restore();
    }
    if !is_root() {
        eprintln!("nitrod: must run as root (start it via systemd: systemctl start nitrod)");
        return ExitCode::FAILURE;
    }
    run();
    ExitCode::SUCCESS
}

/// ExecStopPost failsafe: return the CPU to a safe, balanced state.
fn restore() -> ExitCode {
    if !is_root() {
        return ExitCode::FAILURE;
    }
    // Hand the fans back to firmware control first so nothing is left pinned.
    if fan::supported() {
        let _ = fan::set(fan::FanMode::Auto);
    }
    if platform_profile::supported() {
        if let Some(name) = platform_profile::pick(&["balanced"]) {
            let _ = platform_profile::set(&name);
        }
    }
    let _ = control::apply_balanced();
    println!("nitrod: restored balanced defaults (fans -> firmware auto)");
    ExitCode::SUCCESS
}

fn run() {
    write_pid();
    println!("nitrod: starting (pid {})", std::process::id());

    // Shared "profile override" set by the hotkey thread; 0 means none.
    // Values 1..=4 map to Profile::ALL indices + 1.
    let hotkey_override = Arc::new(AtomicU8::new(0));
    spawn_hotkey_listener(Arc::clone(&hotkey_override));

    let mut last_profile: Option<Profile> = None;
    let mut last_charge: Option<u8> = None;
    let mut last_rgb: Option<rgb::RgbState> = None;
    let mut last_unix = state::now_unix();

    loop {
        let mut cfg = Config::load();

        // Detect a resume-from-suspend via a wall-clock jump larger than a few
        // poll periods, so we can re-apply LED state the firmware may have reset.
        let now_unix = state::now_unix();
        let resumed = now_unix.saturating_sub(last_unix) > cfg.poll_secs.clamp(1, 60) * 3 + 20;
        last_unix = now_unix;

        // A hotkey press cycles the profile and pins it (disables auto-switch).
        let over = hotkey_override.swap(0, Ordering::AcqRel);
        if over != 0 {
            if let Some(p) = Profile::ALL.get((over - 1) as usize).copied() {
                cfg.auto_switch = false;
                cfg.profile = p.as_str().into();
                let _ = cfg.save();
                println!("nitrod: hotkey -> profile {}", p.as_str());
            }
        }

        let ac = telemetry::ac_online();
        let temp = telemetry::cpu_temp_c();
        let guard_active = temp.map(|t| t >= cfg.thermal_guard_c).unwrap_or(false);

        let desired = if guard_active {
            Profile::Quiet
        } else if cfg.auto_switch {
            profile::auto_profile(ac, temp, cfg.thermal_guard_c)
        } else {
            Profile::parse(&cfg.profile).unwrap_or(Profile::Balanced)
        };

        if last_profile != Some(desired) {
            match profile::apply(desired) {
                Ok(()) => {
                    println!("nitrod: applied profile {}", desired.as_str());
                    last_profile = Some(desired);
                }
                Err(e) => eprintln!("nitrod: failed to apply {}: {e}", desired.as_str()),
            }
        }

        // Battery charge limit — only when supported and changed.
        if cfg.charge_limit != last_charge {
            match cfg.charge_limit {
                Some(pct) if battery::supported() => match battery::set_limit(pct) {
                    Ok(()) => {
                        println!("nitrod: charge limit set to {pct}%");
                        last_charge = Some(pct);
                    }
                    Err(e) => eprintln!("nitrod: charge limit: {e}"),
                },
                Some(_) => {
                    // Unsupported on this hardware; record so we don't spam logs.
                    last_charge = cfg.charge_limit;
                }
                None => last_charge = None,
            }
        }

        // Keyboard RGB — re-apply the persisted colour on change or after resume
        // (the firmware can reset the keyboard LEDs across a suspend cycle).
        if rgb::supported() {
            let changed = cfg.rgb != last_rgb;
            if changed || (resumed && cfg.rgb.is_some()) {
                match &cfg.rgb {
                    Some(st) => match st.apply() {
                        Ok(()) => {
                            if changed {
                                println!("nitrod: applied keyboard RGB");
                            }
                            last_rgb = cfg.rgb.clone();
                        }
                        Err(e) => eprintln!("nitrod: RGB apply failed: {e}"),
                    },
                    None => last_rgb = None,
                }
            }
        }

        let _ = state::write(&State {
            profile: desired.as_str().into(),
            auto_switch: cfg.auto_switch,
            cpu_temp_c: temp,
            ac_online: ac,
            thermal_guard_active: guard_active,
            charge_limit: if battery::supported() {
                battery::get_limit()
            } else {
                None
            },
            hotkey_bound: cfg.hotkey_code.is_some(),
            updated_unix: state::now_unix(),
            daemon_pid: std::process::id(),
        });

        thread::sleep(cfg.poll_period());
    }
}

/// Watch the learned hotkey device and publish a profile-cycle request.
fn spawn_hotkey_listener(slot: Arc<AtomicU8>) {
    let cfg = Config::load();
    let Some(code) = cfg.hotkey_code else {
        return;
    };
    let dev: Option<PathBuf> = cfg
        .hotkey_device
        .map(PathBuf::from)
        .or_else(evdev::find_hotkey_device);
    let Some(dev) = dev else {
        eprintln!("nitrod: hotkey code configured but no input device found");
        return;
    };

    thread::spawn(move || {
        println!("nitrod: listening for hotkey code {code} on {}", dev.display());
        let result = evdev::watch_keys(&dev, |pressed| {
            if pressed == code {
                // Compute the next profile from the current config and stash it.
                let current = Profile::parse(&Config::load().profile).unwrap_or(Profile::Balanced);
                let next = current.next();
                let idx = Profile::ALL
                    .iter()
                    .position(|p| *p == next)
                    .map(|i| (i + 1) as u8)
                    .unwrap_or(0);
                slot.store(idx, Ordering::Release);
            }
            true
        });
        if let Err(e) = result {
            eprintln!("nitrod: hotkey listener stopped: {e}");
        }
    });
}

fn write_pid() {
    let _ = std::fs::create_dir_all(state::RUN_DIR);
    let _ = std::fs::write(state::PID_PATH, std::process::id().to_string());
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(str::to_owned))
        })
        .map(|euid| euid == "0")
        .unwrap_or(false)
}
