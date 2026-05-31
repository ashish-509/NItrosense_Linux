//! nitro-gui — a small graphical control panel for Acer Nitro laptops.
//!
//! It is a thin, unprivileged front-end over the existing `nitro` CLI: live
//! sensor/state readings come from `nitro-hal` directly (read-only, no root),
//! while every change (profile, fan, RGB, charge limit) is applied by shelling
//! out to the installed `nitro` binary elevated through pkexec. That keeps all
//! the firmware-validated write logic and persistence in one place and means the
//! GUI itself never runs as root.

use eframe::egui;
use nitro_hal::config::Config;
use nitro_hal::profile::Profile;
use nitro_hal::rgb::{self, RgbState};
use nitro_hal::state::State;
use nitro_hal::telemetry::{self, Telemetry};
use nitro_hal::{battery, fan};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 800.0])
            .with_min_inner_size([480.0, 600.0])
            .with_title("NitroSense"),
        ..Default::default()
    };
    eframe::run_native(
        "NitroSense",
        options,
        Box::new(|cc| Ok(Box::new(NitroApp::new(cc)))),
    )
}

// ---------------------------------------------------------------------------
// Shared state produced by the background poller (all reads are root-free).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Shared {
    loaded: bool,
    telemetry: Option<Telemetry>,
    state: Option<State>,
    daemon_running: bool,
    fan_supported: bool,
    rgb_supported: bool,
    battery_supported: bool,
    fan_readback: Option<String>,
    cfg: Option<Config>,
}

/// Poll hardware/telemetry in the background so the UI thread never blocks.
///
/// To avoid needlessly draining the battery, the poller only samples hardware
/// and asks for repaints while the window is in the foreground. When it is
/// unfocused or hidden it just idles on the `foreground` flag (no sampling, no
/// redraws) and resumes within ~250ms of the window coming back.
fn spawn_poller(ctx: egui::Context, shared: Arc<Mutex<Shared>>, foreground: Arc<AtomicBool>) {
    thread::spawn(move || loop {
        if !foreground.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(250));
            continue;
        }

        // sample() blocks ~500ms to measure CPU utilisation and power deltas.
        let telemetry = telemetry::sample(Duration::from_millis(500));
        let state = nitro_hal::state::read();
        let daemon_running = nitro_hal::state::daemon_running();
        let cfg = Config::load();
        let fan_supported = fan::supported();
        let rgb_supported = rgb::supported();
        let battery_supported = battery::supported();
        let fan_readback = if fan_supported { fan::get() } else { None };

        if let Ok(mut s) = shared.lock() {
            s.telemetry = Some(telemetry);
            s.state = state;
            s.daemon_running = daemon_running;
            s.fan_supported = fan_supported;
            s.rgb_supported = rgb_supported;
            s.battery_supported = battery_supported;
            s.fan_readback = fan_readback;
            s.cfg = Some(cfg);
            s.loaded = true;
        }
        ctx.request_repaint();
        thread::sleep(Duration::from_millis(900));
    });
}

// ---------------------------------------------------------------------------
// Per-frame owned snapshot the widgets read from (avoids locking mid-draw).
// ---------------------------------------------------------------------------

#[derive(Default)]
struct View {
    loaded: bool,
    daemon: bool,
    profile: String,
    auto: bool,
    thermal_guard: bool,
    cpu_temp: Option<f64>,
    cpu_util: Option<f64>,
    cpu_freq: Option<f64>,
    cpu_power: Option<f64>,
    gpu: Option<GpuView>,
    batt: Option<BattView>,
    fan_supported: bool,
    rgb_supported: bool,
    battery_supported: bool,
    fan_readback: String,
    charge_limit: Option<u8>,
    rgb_desc: String,
}

struct GpuView {
    name: String,
    temp: Option<f64>,
    util: Option<f64>,
    clock: Option<f64>,
    power: Option<f64>,
}

struct BattView {
    capacity: Option<u64>,
    status: String,
    power: Option<f64>,
    ac: Option<bool>,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Performance,
    Fans,
    Rgb,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RgbMode {
    Solid,
    Effect,
    Zones,
}

struct ActionResult {
    ok: bool,
    msg: String,
}

struct NitroApp {
    ctx: egui::Context,
    shared: Arc<Mutex<Shared>>,
    view: View,
    nitro: PathBuf,
    tab: Tab,
    initialized: bool,

    // async action plumbing
    pending: Arc<AtomicBool>,
    foreground: Arc<AtomicBool>,
    tx: Sender<ActionResult>,
    rx: Receiver<ActionResult>,
    status: String,
    status_ok: bool,

    // fan controls
    fan_cpu: u8,
    fan_gpu: u8,

    // battery charge limit
    charge_on: bool,
    charge_pct: u8,

    // rgb controls
    rgb_mode: RgbMode,
    color: [u8; 3],
    brightness: u8,
    effect_sel: usize,
    effect_speed: u8,
    effect_dir: u8,
    zones: [[u8; 3]; 4],
    zone_brightness: u8,
}

impl NitroApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let foreground = Arc::new(AtomicBool::new(true));
        spawn_poller(ctx.clone(), Arc::clone(&shared), Arc::clone(&foreground));
        let (tx, rx) = std::sync::mpsc::channel();

        NitroApp {
            ctx,
            shared,
            view: View::default(),
            nitro: find_nitro(),
            tab: Tab::Performance,
            initialized: false,
            pending: Arc::new(AtomicBool::new(false)),
            foreground,
            tx,
            rx,
            status: "Ready.".into(),
            status_ok: true,
            fan_cpu: 60,
            fan_gpu: 60,
            charge_on: false,
            charge_pct: 80,
            rgb_mode: RgbMode::Solid,
            color: [0, 255, 0],
            brightness: 100,
            effect_sel: effect_index_of(4), // "shifting" — a lit mode on this family
            effect_speed: 3,
            effect_dir: 1,
            zones: [[0, 255, 0]; 4],
            zone_brightness: 100,
        }
    }

    /// Build a fresh owned snapshot from the shared poller state.
    fn build_view(&self) -> View {
        let mut v = View::default();
        let s = match self.shared.lock() {
            Ok(s) => s,
            Err(_) => return v,
        };
        v.loaded = s.loaded;
        v.daemon = s.daemon_running;
        v.fan_supported = s.fan_supported;
        v.rgb_supported = s.rgb_supported;
        v.battery_supported = s.battery_supported;
        v.fan_readback = s.fan_readback.clone().unwrap_or_else(|| "?".into());

        if let Some(cfg) = &s.cfg {
            v.profile = cfg.profile.clone();
            v.auto = cfg.auto_switch;
            v.charge_limit = cfg.charge_limit;
            v.rgb_desc = cfg
                .rgb
                .as_ref()
                .map(describe_rgb)
                .unwrap_or_else(|| "none saved".into());
        }
        // The daemon's live state is authoritative when present.
        if let Some(st) = &s.state {
            v.profile = st.profile.clone();
            v.auto = st.auto_switch;
            v.thermal_guard = st.thermal_guard_active;
            if st.charge_limit.is_some() {
                v.charge_limit = st.charge_limit;
            }
        }
        if let Some(t) = &s.telemetry {
            v.cpu_temp = t.cpu.package_temp_c;
            v.cpu_util = t.cpu.utilization_pct;
            v.cpu_freq = t.cpu.avg_freq_mhz;
            v.cpu_power = t.cpu.power_w;
            if let Some(g) = &t.gpu {
                v.gpu = Some(GpuView {
                    name: g.name.clone(),
                    temp: g.temp_c,
                    util: g.utilization_pct,
                    clock: g.clock_mhz,
                    power: g.power_w,
                });
            }
            if let Some(b) = &t.battery {
                v.batt = Some(BattView {
                    capacity: b.capacity_pct,
                    status: b.status.clone().unwrap_or_default(),
                    power: b.power_w,
                    ac: b.ac_online,
                });
            }
        }
        v
    }

    /// Seed the editable controls from the persisted config, once, on first load.
    fn init_controls(&mut self, cfg: &Config) {
        self.charge_on = cfg.charge_limit.is_some();
        if let Some(p) = cfg.charge_limit {
            self.charge_pct = p.clamp(20, 100);
        }
        match &cfg.rgb {
            Some(RgbState::Effect {
                mode,
                speed,
                brightness,
                direction,
                color,
            }) => {
                self.rgb_mode = RgbMode::Effect;
                self.effect_sel = effect_index_of(*mode);
                self.effect_speed = (*speed).min(9);
                self.effect_dir = (*direction).clamp(1, 2);
                self.brightness = (*brightness).min(100);
                if let Some(c) = hex_to_rgb(color) {
                    self.color = c;
                }
            }
            Some(RgbState::Zones { colors, brightness }) => {
                self.rgb_mode = RgbMode::Zones;
                self.zone_brightness = (*brightness).min(100);
                for (i, hex) in colors.iter().enumerate() {
                    if let Some(c) = hex_to_rgb(hex) {
                        self.zones[i] = c;
                    }
                }
                if let Some(c) = hex_to_rgb(&colors[0]) {
                    self.color = c;
                }
            }
            _ => {}
        }
    }

    /// Run `nitro <args>` elevated in the background and report the result.
    fn run(&mut self, args: Vec<String>) {
        if self.pending.load(Ordering::SeqCst) {
            return;
        }
        self.pending.store(true, Ordering::SeqCst);
        self.status_ok = true;
        self.status = format!("Running: nitro {} …", args.join(" "));
        let tx = self.tx.clone();
        let nitro = self.nitro.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let res = run_elevated(&nitro, &args);
            let _ = tx.send(res);
            ctx.request_repaint();
        });
    }

    fn busy(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }
}

impl eframe::App for NitroApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Only let the background poller run while we're actually on screen, so
        // the app is essentially idle when it isn't the focused window.
        let focused = ctx.input(|i| i.focused);
        let was_foreground = self.foreground.swap(focused, Ordering::Relaxed);
        if focused && !was_foreground {
            ctx.request_repaint(); // resume promptly after regaining focus
        }

        // Collect any finished action results.
        while let Ok(r) = self.rx.try_recv() {
            self.pending.store(false, Ordering::SeqCst);
            self.status_ok = r.ok;
            self.status = r.msg;
        }

        // Refresh the per-frame snapshot and one-time-seed the controls.
        self.view = self.build_view();
        if !self.initialized && self.view.loaded {
            let cfg = self.shared.lock().ok().and_then(|s| s.cfg.clone());
            if let Some(cfg) = cfg {
                self.init_controls(&cfg);
            }
            self.initialized = true;
        }

        self.top_bar(ui);
        self.status_bar(ui);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Performance, "  Performance  ");
                ui.selectable_value(&mut self.tab, Tab::Fans, "  Fans  ");
                ui.selectable_value(&mut self.tab, Tab::Rgb, "  Keyboard RGB  ");
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Performance => self.ui_performance(ui),
                Tab::Fans => self.ui_fans(ui),
                Tab::Rgb => self.ui_rgb(ui),
            });
        });
    }
}

// ---------------------------------------------------------------------------
// UI sections
// ---------------------------------------------------------------------------

impl NitroApp {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("NitroSense");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (dot, txt) = if self.view.daemon {
                        (egui::Color32::from_rgb(0x4c, 0xd1, 0x37), "daemon running")
                    } else {
                        (egui::Color32::from_rgb(0xd1, 0x6b, 0x37), "daemon stopped")
                    };
                    ui.label(txt);
                    ui.colored_label(dot, "\u{2b24}");
                });
            });
            ui.horizontal(|ui| {
                let prof = if self.view.profile.is_empty() {
                    "?".to_string()
                } else {
                    self.view.profile.clone()
                };
                ui.label("Profile:");
                ui.strong(cap(&prof));
                if self.view.auto {
                    ui.weak("(auto)");
                }
                ui.separator();
                ui.label("CPU:");
                ui.strong(fmt1(self.view.cpu_temp, "°C"));
                if let Some(g) = &self.view.gpu {
                    ui.separator();
                    ui.label("GPU:");
                    ui.strong(fmt0(g.temp, "°C"));
                }
                if let Some(b) = &self.view.batt {
                    ui.separator();
                    ui.label("Batt:");
                    ui.strong(
                        b.capacity
                            .map(|c| format!("{c}%"))
                            .unwrap_or_else(|| "-".into()),
                    );
                }
            });
            if self.view.thermal_guard {
                ui.colored_label(
                    egui::Color32::from_rgb(0xe0, 0x5a, 0x3a),
                    "⚠ Thermal guard active — CPU forced to Quiet",
                );
            }
            ui.add_space(4.0);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if self.busy() {
                    ui.spinner();
                }
                let color = if self.status_ok {
                    ui.visuals().text_color()
                } else {
                    egui::Color32::from_rgb(0xe0, 0x5a, 0x3a)
                };
                ui.colored_label(color, &self.status);
            });
            ui.add_space(2.0);
        });
    }

    fn ui_performance(&mut self, ui: &mut egui::Ui) {
        let auto = self.view.auto;
        let current = self.view.profile.clone();
        let batt_supported = self.view.battery_supported;
        let charge_limit = self.view.charge_limit;
        let busy = self.busy();

        ui.add_space(6.0);
        ui.heading("Performance profile");
        ui.label("CPU governor / EPP / turbo. Turbo also drives fans to max when the module is loaded.");
        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            for p in Profile::ALL {
                let selected = !auto && current.eq_ignore_ascii_case(p.as_str());
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::selectable(selected, cap(p.as_str())),
                    )
                    .on_hover_text(p.describe())
                    .clicked()
                {
                    self.run(vec!["profile".into(), p.as_str().into()]);
                }
            }
        });

        ui.add_space(8.0);
        let mut auto_toggle = auto;
        if ui
            .add_enabled(!busy, egui::Checkbox::new(&mut auto_toggle, "Auto-switch profile by AC / thermal state"))
            .changed()
        {
            self.run(vec![
                "auto".into(),
                if auto_toggle { "on".into() } else { "off".into() },
            ]);
        }

        ui.add_space(12.0);
        ui.separator();
        ui.heading("Battery charge limit");
        if batt_supported {
            ui.label("Cap charging to protect long-term battery health.");
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(0xd1, 0x9b, 0x37),
                "This firmware/kernel does not expose a charge threshold; the value is saved as a preference only.",
            );
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.charge_on, "Enable");
            ui.add_enabled(
                self.charge_on,
                egui::Slider::new(&mut self.charge_pct, 20..=100).suffix("%"),
            );
            if ui.add_enabled(!busy, egui::Button::new("Apply")).clicked() {
                if self.charge_on {
                    self.run(vec!["charge-limit".into(), self.charge_pct.to_string()]);
                } else {
                    self.run(vec!["charge-limit".into(), "off".into()]);
                }
            }
        });
        if let Some(c) = charge_limit {
            ui.weak(format!("Current: {c}%"));
        }

        ui.add_space(12.0);
        ui.separator();
        ui.heading("Live sensors");
        egui::Grid::new("telemetry")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label("CPU temp");
                ui.label(fmt1(self.view.cpu_temp, "°C"));
                ui.end_row();
                ui.label("CPU usage");
                ui.label(fmt0(self.view.cpu_util, "%"));
                ui.end_row();
                ui.label("CPU freq");
                ui.label(fmt0(self.view.cpu_freq, " MHz"));
                ui.end_row();
                ui.label("CPU power");
                ui.label(fmt1(self.view.cpu_power, " W"));
                ui.end_row();
                if let Some(g) = &self.view.gpu {
                    ui.label(format!("GPU ({})", g.name));
                    ui.label(format!(
                        "{} · {} · {}",
                        fmt0(g.temp, "°C"),
                        fmt0(g.util, "%"),
                        fmt1(g.power, " W")
                    ));
                    ui.end_row();
                    let _ = g.clock;
                }
                if let Some(b) = &self.view.batt {
                    ui.label("Battery");
                    let ac = match b.ac {
                        Some(true) => "AC",
                        Some(false) => "battery",
                        None => "-",
                    };
                    ui.label(format!(
                        "{} · {} · {} · {}",
                        b.capacity
                            .map(|c| format!("{c}%"))
                            .unwrap_or_else(|| "-".into()),
                        if b.status.is_empty() { "-" } else { &b.status },
                        fmt1(b.power, " W"),
                        ac
                    ));
                    ui.end_row();
                }
            });
    }

    fn ui_fans(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Fan control");
        if !self.view.fan_supported {
            module_missing(ui);
            return;
        }
        let busy = self.busy();
        ui.label(format!("Current (cpu,gpu): {}", self.view.fan_readback));
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Auto (firmware)"))
                .clicked()
            {
                self.run(vec!["fan".into(), "auto".into()]);
            }
            if ui.add_enabled(!busy, egui::Button::new("Max")).clicked() {
                self.run(vec!["fan".into(), "max".into()]);
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.label("Manual duty cycle (0 = auto):");
        ui.add(egui::Slider::new(&mut self.fan_cpu, 0..=100).text("CPU fan").suffix("%"));
        ui.add(egui::Slider::new(&mut self.fan_gpu, 0..=100).text("GPU fan").suffix("%"));
        if ui
            .add_enabled(!busy, egui::Button::new("Apply manual speeds"))
            .clicked()
        {
            self.run(vec!["fan".into(), format!("{},{}", self.fan_cpu, self.fan_gpu)]);
        }
        ui.add_space(6.0);
        ui.weak("Values are validated by the firmware. Anything below ~20% may be ignored.");
    }

    fn ui_rgb(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.heading("Keyboard RGB");
        if !self.view.rgb_supported {
            module_missing(ui);
            return;
        }
        let busy = self.busy();
        ui.label(format!("Configured: {}", self.view.rgb_desc));
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.rgb_mode, RgbMode::Solid, "Solid colour");
            ui.selectable_value(&mut self.rgb_mode, RgbMode::Effect, "Animated effect");
            ui.selectable_value(&mut self.rgb_mode, RgbMode::Zones, "Per-zone");
        });
        ui.separator();

        match self.rgb_mode {
            RgbMode::Solid => {
                ui.horizontal(|ui| {
                    ui.label("Colour:");
                    ui.color_edit_button_srgb(&mut self.color);
                });
                ui.add(egui::Slider::new(&mut self.brightness, 0..=100).text("Brightness").suffix("%"));
                if ui
                    .add_enabled(!busy, egui::Button::new("Apply colour"))
                    .clicked()
                {
                    self.run(vec![
                        "rgb".into(),
                        rgb_to_hex(self.color),
                        self.brightness.to_string(),
                    ]);
                }
                ui.add_space(4.0);
                ui.weak(
                    "Note: a truly steady colour is rejected by the AN515-5x firmware, so a solid \
                     colour is shown as a gentle shifting effect (which does light the keys).",
                );
            }
            RgbMode::Effect => {
                ui.horizontal(|ui| {
                    ui.label("Effect:");
                    egui::ComboBox::from_id_salt("effect_combo")
                        .selected_text(effect_label(self.effect_sel))
                        .show_ui(ui, |ui| {
                            for i in 0..rgb::EFFECTS.len() {
                                ui.selectable_value(&mut self.effect_sel, i, effect_label(i));
                            }
                        });
                });
                ui.add(egui::Slider::new(&mut self.effect_speed, 0..=9).text("Speed"));
                ui.horizontal(|ui| {
                    ui.label("Direction:");
                    ui.selectable_value(&mut self.effect_dir, 1, "Forward");
                    ui.selectable_value(&mut self.effect_dir, 2, "Reverse");
                });
                ui.horizontal(|ui| {
                    ui.label("Colour:");
                    ui.color_edit_button_srgb(&mut self.color);
                    ui.weak("(neon / wave ignore colour and cycle the rainbow)");
                });
                ui.add(egui::Slider::new(&mut self.brightness, 0..=100).text("Brightness").suffix("%"));
                if ui
                    .add_enabled(!busy, egui::Button::new("Apply effect"))
                    .clicked()
                {
                    let (name, _mode) = rgb::EFFECTS[self.effect_sel];
                    self.run(vec![
                        "rgb".into(),
                        "effect".into(),
                        name.into(),
                        self.effect_speed.to_string(),
                        self.brightness.to_string(),
                        self.effect_dir.to_string(),
                        rgb_to_hex(self.color),
                    ]);
                }
                ui.add_space(4.0);
                ui.weak(
                    "Lit on this family: neon, wave, shifting, zoom, meteor. \
                     Static / breathing / twinkling may stay dark (firmware limitation).",
                );
            }
            RgbMode::Zones => {
                let names = ["Left", "Centre-left", "Centre-right", "Right"];
                for (i, name) in names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{name}:"));
                        ui.color_edit_button_srgb(&mut self.zones[i]);
                    });
                }
                ui.add(egui::Slider::new(&mut self.zone_brightness, 0..=100).text("Brightness").suffix("%"));
                if ui
                    .add_enabled(!busy, egui::Button::new("Apply zones"))
                    .clicked()
                {
                    let mut args = vec!["rgb".into(), "zones".into()];
                    for z in &self.zones {
                        args.push(rgb_to_hex(*z));
                    }
                    args.push(self.zone_brightness.to_string());
                    self.run(args);
                }
                ui.add_space(4.0);
                ui.weak(
                    "Per-zone static colours use the firmware's static path, which is unreliable on \
                     the AN515-5x family and may leave the keyboard dark.",
                );
            }
        }

        ui.add_space(12.0);
        ui.separator();
        if ui
            .add_enabled(!busy, egui::Button::new("Turn RGB off"))
            .clicked()
        {
            self.run(vec!["rgb".into(), "off".into()]);
        }
    }
}

/// Shared "the kernel module isn't loaded" panel for fan/RGB tabs.
fn module_missing(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.colored_label(
        egui::Color32::from_rgb(0xd1, 0x9b, 0x37),
        "The linuwu-sense kernel module is not loaded, so this feature is unavailable.",
    );
    ui.add_space(6.0);
    ui.label("Install it once (needs kernel headers, sudo and network):");
    ui.code("./scripts/install-kernel-module.sh");
    ui.label("Fans and keyboard stay under firmware control until then.");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Locate the installed `nitro` CLI (next to us, common prefixes, then PATH).
fn find_nitro() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let c = dir.join("nitro");
            if c.is_file() {
                return c;
            }
        }
    }
    for p in ["/usr/local/bin/nitro", "/usr/bin/nitro"] {
        if Path::new(p).is_file() {
            return PathBuf::from(p);
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let c = Path::new(dir).join("nitro");
            if c.is_file() {
                return c;
            }
        }
    }
    PathBuf::from("nitro")
}

/// Run `nitro <args>` as root via pkexec (graphical auth), capturing output.
fn run_elevated(nitro: &Path, args: &[String]) -> ActionResult {
    let mut cmd = if Path::new("/usr/bin/pkexec").exists() {
        let mut c = Command::new("/usr/bin/pkexec");
        c.arg(nitro);
        c
    } else {
        // Fall back to the CLI's own sudo/pkexec elevation.
        Command::new(nitro)
    };
    cmd.args(args);

    match cmd.output() {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if o.status.success() {
                let line = stdout
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
                    .unwrap_or("done");
                ActionResult {
                    ok: true,
                    msg: line.trim().to_string(),
                }
            } else {
                let code = o.status.code().unwrap_or(-1);
                let hint = match code {
                    126 => " (authentication dismissed)",
                    127 => " (not authorised)",
                    _ => "",
                };
                let detail = if !stderr.trim().is_empty() {
                    stderr.trim()
                } else {
                    stdout.trim()
                };
                ActionResult {
                    ok: false,
                    msg: format!("nitro failed (exit {code}){hint}: {detail}"),
                }
            }
        }
        Err(e) => ActionResult {
            ok: false,
            msg: format!("could not launch {}: {e}", nitro.display()),
        },
    }
}

fn describe_rgb(state: &RgbState) -> String {
    match state {
        RgbState::Off => "off".into(),
        RgbState::Zones { colors, brightness } => {
            if colors.iter().all(|c| c == &colors[0]) {
                format!("solid #{} @ {}%", colors[0], brightness)
            } else {
                format!("zones {} @ {}%", colors.join("/"), brightness)
            }
        }
        RgbState::Effect {
            mode,
            brightness,
            color,
            ..
        } => format!("effect {} #{} @ {}%", rgb::effect_name(*mode), color, brightness),
    }
}

fn effect_index_of(mode: u8) -> usize {
    rgb::EFFECTS
        .iter()
        .position(|(_, m)| *m == mode)
        .unwrap_or(0)
}

fn effect_label(idx: usize) -> String {
    let (name, mode) = rgb::EFFECTS[idx];
    let dark = matches!(mode, 0 | 1 | 7);
    if dark {
        format!("{name} (may stay dark)")
    } else {
        name.to_string()
    }
}

fn hex_to_rgb(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some([
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ])
}

fn rgb_to_hex(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn cap(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn fmt1(v: Option<f64>, unit: &str) -> String {
    v.map(|x| format!("{x:.1}{unit}")).unwrap_or_else(|| "-".into())
}

fn fmt0(v: Option<f64>, unit: &str) -> String {
    v.map(|x| format!("{x:.0}{unit}")).unwrap_or_else(|| "-".into())
}
