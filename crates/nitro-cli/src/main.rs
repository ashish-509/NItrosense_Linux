use nitro_hal::config::Config;
use nitro_hal::profile::{self, Profile};
use nitro_hal::{battery, control, evdev, fan, rgb, state};
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    match cmd.as_deref() {
        Some("activate") => set_profile(Profile::Performance),
        Some("deactivate") => set_profile(Profile::Balanced),
        Some("profile") => cmd_profile(rest.first().map(String::as_str)),
        Some("auto") => cmd_auto(rest.first().map(String::as_str)),
        Some("charge-limit") | Some("charge") => cmd_charge_limit(rest.first().map(String::as_str)),
        Some("learn-key") => cmd_learn_key(),
        Some("fan") => cmd_fan(&rest),
        Some("rgb") => cmd_rgb(&rest),
        Some("status") => cmd_status(),
        Some("monitor") => cmd_monitor(),
        Some("help") | Some("-h") | Some("--help") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("nitro: unknown command '{other}' (try: nitro help)");
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!("nitro - Acer Nitro control\n");
    println!("USAGE:");
    println!("  nitro <command> [args]\n");
    println!("PROFILES (CPU/cpufreq - verified):");
    println!("  activate                 shortcut for: profile performance");
    println!("  deactivate               shortcut for: profile balanced");
    println!("  profile <name>           quiet | balanced | performance | turbo");
    println!("  auto <on|off>            auto-switch profile by AC/thermal (needs daemon)");
    println!("STATUS:");
    println!("  status                   current mode, daemon state, live sensors");
    println!("  monitor                  stream live sensors (Ctrl-C to stop)");
    println!("OTHER:");
    println!("  charge-limit <pct|off>   battery charge cap (if firmware exposes it)");
    println!("  learn-key                capture the NitroSense key to cycle profiles");
    println!("  fan [auto|max|C,G]       fan speed, percent 0-100 (needs linuwu-sense module)");
    println!("  rgb <off|RRGGBB|brightness|zones|effect>  keyboard RGB (needs linuwu-sense)\n");
    println!("Commands that change hardware re-run with sudo automatically.");
    println!("Fan, RGB and Acer firmware thermal profiles use the linuwu-sense kernel");
    println!("module (firmware-validated WMI). Install it once with:");
    println!("  ./scripts/install-kernel-module.sh   (kernel headers + sudo + network)");
}

fn set_profile(p: Profile) -> ExitCode {
    ensure_root();
    let mut cfg = Config::load();
    cfg.profile = p.as_str().into();
    cfg.auto_switch = false;
    if let Err(e) = cfg.save() {
        eprintln!("nitro: warning: could not save config: {e}");
    }
    match profile::apply(p) {
        Ok(()) => {
            println!("nitro: profile -> {}", p.describe());
            print_cpu_line(&control::status());
            if !state::daemon_running() {
                println!("  (daemon not running; applied once - enable it for persistence/auto)");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nitro: failed to apply profile: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_profile(arg: Option<&str>) -> ExitCode {
    match arg.and_then(Profile::parse) {
        Some(p) => set_profile(p),
        None => {
            eprintln!("nitro: usage: nitro profile <quiet|balanced|performance|turbo>");
            ExitCode::from(2)
        }
    }
}

fn cmd_auto(arg: Option<&str>) -> ExitCode {
    let on = match arg {
        Some("on") | Some("true") | Some("1") => true,
        Some("off") | Some("false") | Some("0") => false,
        _ => {
            eprintln!("nitro: usage: nitro auto <on|off>");
            return ExitCode::from(2);
        }
    };
    ensure_root();
    let mut cfg = Config::load();
    cfg.auto_switch = on;
    if let Err(e) = cfg.save() {
        eprintln!("nitro: could not save config: {e}");
        return ExitCode::FAILURE;
    }
    println!("nitro: auto-switch {}", if on { "ENABLED" } else { "disabled" });
    if on && !state::daemon_running() {
        println!("  note: start the daemon for auto-switching: sudo systemctl enable --now nitrod");
    }
    ExitCode::SUCCESS
}

fn cmd_charge_limit(arg: Option<&str>) -> ExitCode {
    match arg {
        None => {
            let cur = battery::get_limit();
            println!(
                "nitro: charge limit {} (hardware support: {})",
                cur.map(|v| format!("{v}%")).unwrap_or_else(|| "unset".into()),
                if battery::supported() { "yes" } else { "no" }
            );
            ExitCode::SUCCESS
        }
        Some("off") | Some("none") => {
            ensure_root();
            let mut cfg = Config::load();
            cfg.charge_limit = None;
            let _ = cfg.save();
            println!("nitro: charge limit cleared");
            ExitCode::SUCCESS
        }
        Some(v) => match v.parse::<u8>() {
            Ok(pct) => {
                ensure_root();
                let mut cfg = Config::load();
                cfg.charge_limit = Some(pct);
                if let Err(e) = cfg.save() {
                    eprintln!("nitro: could not save config: {e}");
                    return ExitCode::FAILURE;
                }
                if !battery::supported() {
                    println!("nitro: saved {pct}% preference, but this kernel/firmware does not");
                    println!("       expose a charge threshold yet, so it cannot be enforced.");
                    return ExitCode::SUCCESS;
                }
                match battery::set_limit(pct) {
                    Ok(()) => {
                        println!("nitro: battery charge limit set to {pct}%");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("nitro: {e}");
                        ExitCode::FAILURE
                    }
                }
            }
            Err(_) => {
                eprintln!("nitro: usage: nitro charge-limit <20-100|off>");
                ExitCode::from(2)
            }
        },
    }
}

fn cmd_learn_key() -> ExitCode {
    ensure_root();
    let dev = match evdev::find_hotkey_device() {
        Some(d) => d,
        None => {
            eprintln!("nitro: could not find an Acer hotkey input device");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "nitro: press your NitroSense key now (listening on {})...",
        dev.display()
    );
    match evdev::wait_for_keypress(&dev) {
        Ok(code) => {
            let mut cfg = Config::load();
            cfg.hotkey_device = Some(dev.to_string_lossy().into_owned());
            cfg.hotkey_code = Some(code);
            if let Err(e) = cfg.save() {
                eprintln!("nitro: could not save config: {e}");
                return ExitCode::FAILURE;
            }
            println!("nitro: learned keycode {code}. Restart the daemon to bind it:");
            println!("       sudo systemctl restart nitrod");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nitro: failed to read key: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_fan(args: &[String]) -> ExitCode {
    if !fan::supported() {
        println!("nitro: fan control unavailable - the linuwu-sense kernel module is not loaded.");
        println!("  Install it once with: ./scripts/install-kernel-module.sh");
        println!("  (needs kernel headers, sudo and network). Fans stay under firmware control.");
        return ExitCode::SUCCESS;
    }
    match args.first().map(String::as_str) {
        None => {
            println!("fan (cpu,gpu): {}", fan::get().unwrap_or_else(|| "?".into()));
            println!("usage: nitro fan <auto|max|CPU,GPU>   (percent 0-100, 0 = auto)");
            ExitCode::SUCCESS
        }
        Some(arg) => {
            let mode = match arg {
                "auto" => fan::FanMode::Auto,
                "max" => fan::FanMode::Max,
                spec => match parse_pair(spec) {
                    Some((cpu, gpu)) => fan::FanMode::Manual { cpu, gpu },
                    None => {
                        eprintln!("nitro: invalid fan spec '{spec}' (use auto, max, or CPU,GPU)");
                        return ExitCode::from(2);
                    }
                },
            };
            ensure_root();
            match fan::set(mode) {
                Ok(()) => {
                    println!("nitro: fan -> {}", fan::get().unwrap_or_else(|| "?".into()));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("nitro: fan set failed: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn parse_pair(spec: &str) -> Option<(u8, u8)> {
    let (a, b) = spec.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn cmd_rgb(args: &[String]) -> ExitCode {
    if !rgb::supported() {
        if nitro_hal::acer::module_loaded() {
            println!("nitro: keyboard RGB group (four_zoned_kb) is not present.");
            println!("  The linuwu-sense module is loaded but the RGB interface is off for this model.");
            println!("  Re-run ./scripts/install-kernel-module.sh to build in the AN515-56 quirk,");
            println!("  then check: sudo dmesg | grep -i acer");
        } else {
            println!("nitro: keyboard RGB unavailable - the linuwu-sense kernel module is not loaded.");
            println!("  Install it once with: ./scripts/install-kernel-module.sh");
            println!("  (needs kernel headers, sudo and network).");
        }
        return ExitCode::SUCCESS;
    }
    match args.first().map(String::as_str) {
        None => {
            match Config::load().rgb {
                Some(st) => println!("rgb (configured): {}", describe_rgb(&st)),
                None => println!("rgb (configured): <none saved>"),
            }
            println!("  four_zone_mode: {}", rgb::get_four_zone().unwrap_or_else(|| "?".into()));
            println!("  per_zone_mode : {}", rgb::get().unwrap_or_else(|| "?".into()));
            println!("usage:");
            println!("  nitro rgb off");
            println!("  nitro rgb <RRGGBB> [brightness]   (solid colour, shown as a shifting effect)");
            println!("  nitro rgb brightness <0-100>");
            println!("  nitro rgb zones <c1> <c2> <c3> <c4> [brightness]");
            println!("  nitro rgb effect <name|0-7> <speed 0-9> <brightness 0-100> <dir 1-2> <RRGGBB>");
            println!("  effects: {}", rgb::effect_names().join(", "));
            println!("  note: steady static/breathing colour is unsupported by this firmware; effects work.");
            ExitCode::SUCCESS
        }
        Some("off") => apply_and_save_rgb(rgb::RgbState::Off),
        Some("brightness") => {
            let b = match args.get(1).and_then(|v| v.parse::<u8>().ok()) {
                Some(v) => v.min(100),
                None => {
                    eprintln!("nitro: usage: nitro rgb brightness <0-100>");
                    return ExitCode::from(2);
                }
            };
            ensure_root();
            let base = Config::load().rgb.or_else(|| {
                rgb::get()
                    .and_then(|s| rgb::parse_zone_string(&s))
                    .map(|(zones, _)| rgb::RgbState::Zones {
                        colors: zones.map(|z| z.hex()),
                        brightness: 100,
                    })
            });
            match base {
                Some(state) => apply_and_save_rgb(state.with_brightness(b)),
                None => {
                    eprintln!("nitro: no current RGB colour to adjust - set one first, e.g. nitro rgb ff0000");
                    ExitCode::from(2)
                }
            }
        }
        Some("zones") => {
            let z = &args[1..];
            if z.len() < 4 {
                eprintln!("nitro: usage: nitro rgb zones <c1> <c2> <c3> <c4> [brightness]");
                return ExitCode::from(2);
            }
            let mut colors = [String::new(), String::new(), String::new(), String::new()];
            for (i, c) in z[..4].iter().enumerate() {
                match rgb::Rgb::parse(c) {
                    Some(col) => colors[i] = col.hex(),
                    None => {
                        eprintln!("nitro: invalid colour '{c}'");
                        return ExitCode::from(2);
                    }
                }
            }
            let brightness = z.get(4).and_then(|b| b.parse::<u8>().ok()).unwrap_or(100);
            apply_and_save_rgb(rgb::RgbState::Zones { colors, brightness })
        }
        Some("effect") => {
            let e = &args[1..];
            if e.len() < 5 {
                eprintln!("nitro: usage: nitro rgb effect <name|0-7> <speed 0-9> <brightness 0-100> <dir 1-2> <RRGGBB>");
                eprintln!("  effects: {}", rgb::effect_names().join(", "));
                return ExitCode::from(2);
            }
            let mode = match rgb::effect_mode(&e[0]) {
                Some(m) => m,
                None => {
                    eprintln!("nitro: unknown effect '{}' (try: {})", e[0], rgb::effect_names().join(", "));
                    return ExitCode::from(2);
                }
            };
            let speed = e[1].parse::<u8>().unwrap_or(0);
            let brightness = e[2].parse::<u8>().unwrap_or(100);
            let dir = e[3].parse::<u8>().unwrap_or(1);
            let color = match rgb::Rgb::parse(&e[4]) {
                Some(c) => c,
                None => {
                    eprintln!("nitro: invalid colour '{}'", e[4]);
                    return ExitCode::from(2);
                }
            };
            apply_and_save_rgb(rgb::RgbState::Effect {
                mode,
                speed,
                brightness,
                direction: dir,
                color: color.hex(),
            })
        }
        Some(color) => match rgb::Rgb::parse(color) {
            Some(c) => {
                let brightness = args.get(1).and_then(|b| b.parse::<u8>().ok()).unwrap_or(100);
                // Steady "static" colour (mode 0) is rejected by the AN515-5x
                // firmware (the keyboard goes dark), so render a solid colour as
                // a gentle "shifting" effect, which does light the keys.
                println!(
                    "nitro: applying #{} as a shifting effect (steady static colour is unsupported on this firmware)",
                    c.hex()
                );
                apply_and_save_rgb(rgb::RgbState::Effect {
                    mode: 4, // shifting
                    speed: 3,
                    brightness,
                    direction: 1,
                    color: c.hex(),
                })
            }
            None => {
                eprintln!("nitro: invalid colour '{color}' (expected RRGGBB, off, brightness, zones or effect)");
                ExitCode::from(2)
            }
        },
    }
}

/// One-line human description of a persisted RGB state.
fn describe_rgb(state: &rgb::RgbState) -> String {
    match state {
        rgb::RgbState::Off => "off".into(),
        rgb::RgbState::Zones { colors, brightness } => {
            if colors.iter().all(|c| c == &colors[0]) {
                format!("solid #{} @ {}%", colors[0], brightness)
            } else {
                format!("zones {} @ {}%", colors.join("/"), brightness)
            }
        }
        rgb::RgbState::Effect {
            mode,
            brightness,
            color,
            ..
        } => format!(
            "effect {} #{} @ {}%",
            rgb::effect_name(*mode),
            color,
            brightness
        ),
    }
}

/// Apply an RGB state to the hardware and persist it so the daemon re-applies it
/// on boot/resume. Re-execs via sudo (writes sysfs + /etc/nitro/config.json).
fn apply_and_save_rgb(state: rgb::RgbState) -> ExitCode {
    ensure_root();
    match state.apply() {
        Ok(()) => {
            let mut cfg = Config::load();
            cfg.rgb = Some(state);
            if let Err(e) = cfg.save() {
                eprintln!("nitro: RGB applied but not saved: {e}");
            }
            println!("nitro: keyboard RGB updated");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("nitro: RGB set failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_status() -> ExitCode {
    let s = control::status();
    let cfg = Config::load();
    let daemon = state::daemon_running();
    println!("nitro status");
    println!("  daemon    : {}", if daemon { "running" } else { "stopped" });
    match state::read() {
        Some(st) => {
            println!(
                "  profile   : {}{}",
                st.profile,
                if st.auto_switch { " (auto)" } else { "" }
            );
            if st.thermal_guard_active {
                println!("  THERMAL GUARD ACTIVE (CPU forced to quiet)");
            }
        }
        None => println!(
            "  profile   : {}{}",
            cfg.profile,
            if cfg.auto_switch { " (auto)" } else { "" }
        ),
    }
    println!("  driver    : {}", s.driver.as_deref().unwrap_or("?"));
    println!("  governor  : {}", s.governor.as_deref().unwrap_or("?"));
    println!("  epp       : {}", s.epp.as_deref().unwrap_or("?"));
    println!("  turbo     : {}", turbo_str(s.turbo_enabled));
    println!("  governors : {}", s.available_governors.join(", "));
    println!(
        "  charge    : {} (hw support: {})",
        cfg.charge_limit.map(|v| format!("{v}%")).unwrap_or_else(|| "unset".into()),
        if battery::supported() { "yes" } else { "no" },
    );
    println!(
        "  hotkey    : {}",
        cfg.hotkey_code.map(|c| format!("bound (code {c})")).unwrap_or_else(|| "not learned".into()),
    );
    println!(
        "  fan       : {}",
        if fan::supported() {
            fan::get().unwrap_or_else(|| "?".into())
        } else {
            "n/a (linuwu-sense not loaded)".into()
        },
    );
    println!(
        "  rgb       : {}",
        if rgb::supported() {
            rgb::get().unwrap_or_else(|| "?".into())
        } else {
            "n/a (linuwu-sense not loaded)".into()
        },
    );
    println!();
    print!("{}", render_telemetry(&control_snapshot()));
    ExitCode::SUCCESS
}

fn cmd_monitor() -> ExitCode {
    loop {
        print!("{}", render_telemetry(&control_snapshot()));
    }
}

fn control_snapshot() -> nitro_hal::Telemetry {
    nitro_hal::telemetry::sample(Duration::from_millis(500))
}

fn print_cpu_line(s: &control::CpuStatus) {
    println!(
        "  governor={} epp={} turbo={}",
        s.governor.as_deref().unwrap_or("?"),
        s.epp.as_deref().unwrap_or("?"),
        turbo_str(s.turbo_enabled),
    );
}

fn turbo_str(t: Option<bool>) -> &'static str {
    match t {
        Some(true) => "on",
        Some(false) => "off",
        None => "?",
    }
}

fn render_telemetry(t: &nitro_hal::Telemetry) -> String {
    use std::fmt::Write as _;
    let mut o = String::new();
    let f1 = |v: Option<f64>, u: &str| v.map(|x| format!("{x:.1}{u}")).unwrap_or_else(|| "-".into());
    let f0 = |v: Option<f64>, u: &str| v.map(|x| format!("{x:.0}{u}")).unwrap_or_else(|| "-".into());

    let c = &t.cpu;
    let _ = writeln!(
        o,
        "cpu   temp {}  util {}  freq {}/{} MHz  power {}",
        f1(c.package_temp_c, "C"),
        f0(c.utilization_pct, "%"),
        f0(c.avg_freq_mhz, ""),
        f0(c.max_freq_mhz, ""),
        f1(c.power_w, "W"),
    );
    match &t.gpu {
        Some(g) => {
            let _ = writeln!(
                o,
                "gpu   {} temp {} util {} clock {} power {}",
                g.name,
                f0(g.temp_c, "C"),
                f0(g.utilization_pct, "%"),
                f0(g.clock_mhz, "MHz"),
                f1(g.power_w, "W"),
            );
        }
        None => {
            let _ = writeln!(o, "gpu   n/a");
        }
    }
    match &t.battery {
        Some(b) => {
            let _ = writeln!(
                o,
                "batt  {}%  {}  {}  ac={}",
                b.capacity_pct.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                b.status.as_deref().unwrap_or("-"),
                f1(b.power_w, "W"),
                b.ac_online.map(|v| if v { "yes" } else { "no" }).unwrap_or("-"),
            );
        }
        None => {
            let _ = writeln!(o, "batt  n/a");
        }
    }
    if t.fans.is_empty() {
        let _ = writeln!(o, "fans  (RPM interface not yet verified on this hardware)");
    } else {
        for fan in &t.fans {
            let _ = writeln!(o, "fan   {} {} rpm", fan.name, fan.rpm);
        }
    }
    let _ = writeln!(o, "----");
    o
}

fn ensure_root() {
    if is_root() {
        return;
    }
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().unwrap_or_else(|_| "nitro".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    eprintln!("nitro: requesting root for hardware access...");
    for helper in ["sudo", "pkexec"] {
        // exec replaces this process; it only returns if the helper is missing.
        let _ = std::process::Command::new(helper).arg(&exe).args(&args).exec();
    }
    eprintln!("nitro: could not elevate; run: sudo nitro {}", args.join(" "));
    std::process::exit(1);
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
