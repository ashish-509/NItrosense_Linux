use nitro_hal::Report;
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let mut json = false;
    let mut output: Option<String> = None;
    let mut telemetry = false;
    let mut watch = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--telemetry" => telemetry = true,
            "--watch" => watch = true,
            "-o" | "--output" => match args.next() {
                Some(path) => output = Some(path),
                None => {
                    eprintln!("error: {arg} requires a path");
                    return ExitCode::from(2);
                }
            },
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("error: unknown argument '{other}' (try --help)");
                return ExitCode::from(2);
            }
        }
    }

    if telemetry {
        return run_telemetry(json, watch);
    }

    let report = nitro_hal::run();
    let rendered = if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: serialization failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        render_text(&report)
    };

    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, rendered) {
                eprintln!("error: writing {path}: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("report written to {path}");
        }
        None => println!("{rendered}"),
    }
    ExitCode::SUCCESS
}

fn print_help() {
    println!("nitro-discover - read-only Acer Nitro hardware discovery\n");
    println!("USAGE:");
    println!("  nitro-discover [--json] [-o <file>]");
    println!("  nitro-discover --telemetry [--watch] [--json]\n");
    println!("OPTIONS:");
    println!("  --json          emit machine-readable JSON");
    println!("  --telemetry     print a live sensor snapshot instead of the report");
    println!("  --watch         with --telemetry, refresh continuously (Ctrl-C to stop)");
    println!("  -o, --output    write the report to <file> instead of stdout");
    println!("  -h, --help      show this help\n");
    println!("Run with sudo for the EC dump and ACPI table listing.");
}

fn run_telemetry(json: bool, watch: bool) -> ExitCode {
    let window = Duration::from_millis(500);
    loop {
        let snapshot = nitro_hal::telemetry::sample(window);
        if json {
            match serde_json::to_string(&snapshot) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("error: serialization failed: {e}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            print!("{}", render_telemetry(&snapshot));
        }
        if !watch {
            return ExitCode::SUCCESS;
        }
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

fn render_text(r: &Report) -> String {
    use std::fmt::Write as _;
    let mut o = String::new();
    let yn = |b: bool| if b { "yes" } else { "no" };
    let opt = |v: &Option<String>| v.as_deref().unwrap_or("?").to_owned();

    let _ = writeln!(o, "Nitro-Linux hardware discovery");
    let _ = writeln!(
        o,
        "generated: unix {}   root: {}\n",
        r.timestamp_unix,
        yn(r.running_as_root)
    );

    let c = &r.capabilities;
    let _ = writeln!(o, "== capability summary (read-only verified) ==");
    let _ = writeln!(o, "  temperatures readable : {}", yn(c.temperatures_readable));
    let _ = writeln!(o, "  fan rpm readable      : {}", yn(c.fan_rpm_readable));
    let _ = writeln!(o, "  pwm channels present  : {}", yn(c.pwm_present));
    let _ = writeln!(o, "  power readable        : {}", yn(c.power_readable));
    let _ = writeln!(o, "  acer-wmi platform     : {}", yn(c.acer_wmi_platform_present));
    let _ = writeln!(o, "  acer gaming WMI GUID  : {}", yn(c.acer_gaming_wmi_guid_present));
    let _ = writeln!(o, "  EC debugfs readable   : {}", yn(c.ec_debugfs_readable));
    let _ = writeln!(o, "  RGB led (multicolor)  : {}", yn(c.rgb_led_present));
    let _ = writeln!(o, "  kbd backlight led     : {}", yn(c.kbd_backlight_present));
    let _ = writeln!(o, "  battery charge limit  : {}", yn(c.battery_charge_limit_supported));
    let _ = writeln!(o, "  hotkey candidates     : {}", c.hotkey_candidates);
    let _ = writeln!(o, "  nvidia dGPU present    : {}\n", yn(c.nvidia_dgpu_present));

    let d = &r.dmi;
    let _ = writeln!(o, "== system ==");
    let _ = writeln!(o, "  vendor/model : {} / {}", opt(&d.sys_vendor), opt(&d.product_name));
    let _ = writeln!(o, "  family/board : {} / {}", opt(&d.product_family), opt(&d.board_name));
    let _ = writeln!(
        o,
        "  bios         : {} {} ({})",
        opt(&d.bios_vendor),
        opt(&d.bios_version),
        opt(&d.bios_date)
    );
    let _ = writeln!(o, "  kernel       : {}", opt(&r.kernel.release));
    let _ = writeln!(
        o,
        "  cpu          : {} ({} threads)\n",
        opt(&r.cpu.model),
        r.cpu.cores_logical
    );

    let _ = writeln!(o, "== display controllers ==");
    for g in &r.pci_display {
        let _ = writeln!(
            o,
            "  {} [{}:{}] {} driver={}",
            g.slot,
            opt(&g.vendor),
            opt(&g.device),
            g.vendor_name.unwrap_or("?"),
            opt(&g.driver)
        );
    }
    o.push('\n');

    let _ = writeln!(o, "== hwmon ==");
    for chip in &r.hwmon {
        let _ = writeln!(o, "  [{}] {}", opt(&chip.name), chip.path);
        for t in &chip.temps {
            let val = t.celsius.map(|v| format!("{v:.1}C")).unwrap_or_else(|| "?".into());
            let _ = writeln!(o, "     temp{:<2} {:>7}  {}", t.index, val, t.label.as_deref().unwrap_or(""));
        }
        for f in &chip.fans {
            let val = f.rpm.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
            let _ = writeln!(o, "     fan{:<3} {:>6} rpm  {}", f.index, val, f.label.as_deref().unwrap_or(""));
        }
        for p in &chip.pwms {
            let raw = p.raw.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
            let en = p.enable.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
            let _ = writeln!(o, "     pwm{:<3} raw={} enable={}", p.index, raw, en);
        }
        for p in &chip.powers {
            let _ = writeln!(o, "     power{} {:.2}W  {}", p.index, p.watts.unwrap_or(0.0), p.label.as_deref().unwrap_or(""));
        }
    }
    o.push('\n');

    let _ = writeln!(o, "== wmi devices ==");
    for w in &r.wmi {
        let tag = w.known.map(|k| format!("  <= {k}")).unwrap_or_default();
        let _ = writeln!(o, "  {} guid={} driver={}{}", w.device, opt(&w.guid), opt(&w.driver), tag);
    }
    if let Some(ap) = &r.acer_platform {
        let _ = writeln!(o, "  acer-wmi platform: {}", ap.path);
        for (k, v) in &ap.attributes {
            let _ = writeln!(o, "     {k} = {v}");
        }
    }
    o.push('\n');

    let _ = writeln!(o, "== leds ==");
    for l in &r.leds {
        let mark = if l.is_keyboard_candidate { " *kbd" } else { "" };
        let _ = writeln!(
            o,
            "  {}{} bright={}/{} multi_index={} multi_intensity={}",
            l.name,
            mark,
            l.brightness.map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
            l.max_brightness.map(|v| v.to_string()).unwrap_or_else(|| "?".into()),
            l.multi_index.as_deref().unwrap_or("-"),
            l.multi_intensity.as_deref().unwrap_or("-"),
        );
    }
    o.push('\n');

    let _ = writeln!(o, "== power supplies ==");
    for p in &r.power {
        let _ = writeln!(
            o,
            "  {} type={} online={} status={} cap={} charge_end={} limit_supported={}",
            p.name,
            opt(&p.kind),
            p.online.map(|b| yn(b).to_string()).unwrap_or_else(|| "-".into()),
            opt(&p.status),
            p.capacity.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            p.charge_control_end_threshold.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
            yn(p.charge_limit_supported),
        );
    }
    o.push('\n');

    let _ = writeln!(o, "== input devices (hotkey candidates marked *) ==");
    for dev in &r.inputs {
        let mark = if dev.hotkey_candidate { " *" } else { "  " };
        let _ = writeln!(
            o,
            " {}{} [{}] keys={}",
            mark,
            dev.name.as_deref().unwrap_or("?"),
            dev.handlers.join(","),
            yn(dev.ev_keys),
        );
    }
    o.push('\n');

    let _ = writeln!(o, "== acpi tables ==");
    let tables = if r.acpi_tables.is_empty() {
        "(none / need root)".to_string()
    } else {
        r.acpi_tables.join(" ")
    };
    let _ = writeln!(o, "  {tables}\n");

    let _ = writeln!(o, "== embedded controller ==");
    let ws = r.ec.write_support.map(|b| yn(b).to_string()).unwrap_or_else(|| "?".into());
    let _ = writeln!(
        o,
        "  debugfs_present={} readable={} write_support={}  {}",
        yn(r.ec.debugfs_present),
        yn(r.ec.readable),
        ws,
        r.ec.note.as_deref().unwrap_or(""),
    );
    if let Some(hex) = &r.ec.dump_hex {
        let _ = writeln!(o, "  EC 256-byte map:");
        for line in hex.lines() {
            let _ = writeln!(o, "    {line}");
        }
    }
    o.push('\n');

    if !r.notes.is_empty() {
        let _ = writeln!(o, "== notes ==");
        for n in &r.notes {
            let _ = writeln!(o, "  - {n}");
        }
        o.push('\n');
    }

    let _ = writeln!(o, "== next steps ==");
    let _ = writeln!(o, "  1. If EC/ACPI show 'need root', re-run: sudo nitro-discover");
    let _ = writeln!(o, "  2. Capture the NitroSense key (interactive, live keypress):");
    let _ = writeln!(o, "       sudo acpi_listen      # press the key 3-4x");
    let _ = writeln!(o, "       sudo evtest           # choose a *-marked device, press the key");
    let _ = writeln!(o, "  3. Paste this report plus the key-capture output for analysis.");

    o
}
