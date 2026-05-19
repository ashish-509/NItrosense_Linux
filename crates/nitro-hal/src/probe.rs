use crate::sysfs::read_trim;
use crate::{dmi, ec, hwmon, input, leds, pci, power, wmi};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Kernel {
    pub release: Option<String>,
    pub version: Option<String>,
    pub arch: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Cpu {
    pub model: Option<String>,
    pub vendor: Option<String>,
    pub cores_logical: usize,
}

#[derive(Debug, Serialize)]
pub struct CapabilitySummary {
    pub temperatures_readable: bool,
    pub fan_rpm_readable: bool,
    pub pwm_present: bool,
    pub power_readable: bool,
    pub acer_wmi_platform_present: bool,
    pub acer_gaming_wmi_guid_present: bool,
    pub ec_debugfs_readable: bool,
    pub rgb_led_present: bool,
    pub kbd_backlight_present: bool,
    pub battery_charge_limit_supported: bool,
    pub hotkey_candidates: usize,
    pub nvidia_dgpu_present: bool,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub timestamp_unix: u64,
    pub running_as_root: bool,
    pub dmi: dmi::DmiInfo,
    pub kernel: Kernel,
    pub cpu: Cpu,
    pub pci_display: Vec<pci::PciDevice>,
    pub hwmon: Vec<hwmon::HwmonChip>,
    pub wmi: Vec<wmi::WmiDevice>,
    pub acer_platform: Option<wmi::AcerPlatform>,
    pub ec: ec::EcInfo,
    pub leds: Vec<leds::LedDevice>,
    pub power: Vec<power::PowerSupply>,
    pub inputs: Vec<input::InputDevice>,
    pub acpi_tables: Vec<String>,
    pub capabilities: CapabilitySummary,
    pub notes: Vec<String>,
}

pub fn run() -> Report {
    let mut notes = Vec::new();
    let running_as_root = running_as_root();
    if !running_as_root {
        notes.push(
            "Not running as root: EC dump and ACPI table listing are unavailable. \
             Re-run with sudo for a complete report."
                .into(),
        );
    }

    let dmi = dmi::read();
    let kernel = read_kernel();
    let cpu = read_cpu();
    let pci_display = pci::read_display();
    let hwmon = hwmon::read();
    let wmi = wmi::read();
    let acer_platform = wmi::acer_platform();
    let ec = ec::read();
    let leds = leds::read();
    let power = power::read();
    let inputs = input::read();
    let acpi_tables = read_acpi_tables(&mut notes);

    let capabilities = CapabilitySummary {
        temperatures_readable: hwmon.iter().any(|c| c.temps.iter().any(|t| t.celsius.is_some())),
        fan_rpm_readable: hwmon.iter().any(|c| c.fans.iter().any(|f| f.rpm.is_some())),
        pwm_present: hwmon.iter().any(|c| !c.pwms.is_empty()),
        power_readable: hwmon.iter().any(|c| c.powers.iter().any(|p| p.watts.is_some())),
        acer_wmi_platform_present: acer_platform.is_some(),
        acer_gaming_wmi_guid_present: wmi.iter().any(|d| d.known == Some(wmi::GAMING_LABEL)),
        ec_debugfs_readable: ec.readable,
        rgb_led_present: leds.iter().any(|l| l.multi_intensity.is_some()),
        kbd_backlight_present: leds.iter().any(|l| l.is_keyboard_candidate),
        battery_charge_limit_supported: power.iter().any(|p| p.charge_limit_supported),
        hotkey_candidates: inputs.iter().filter(|i| i.hotkey_candidate).count(),
        nvidia_dgpu_present: pci_display.iter().any(|d| d.vendor_name == Some("NVIDIA")),
    };

    Report {
        timestamp_unix: now(),
        running_as_root,
        dmi,
        kernel,
        cpu,
        pci_display,
        hwmon,
        wmi,
        acer_platform,
        ec,
        leds,
        power,
        inputs,
        acpi_tables,
        capabilities,
        notes,
    }
}

fn running_as_root() -> bool {
    read_trim("/proc/self/status")
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(str::to_owned))
        })
        .map(|euid| euid == "0")
        .unwrap_or(false)
}

fn read_kernel() -> Kernel {
    Kernel {
        release: read_trim("/proc/sys/kernel/osrelease"),
        version: read_trim("/proc/version"),
        arch: std::env::consts::ARCH,
    }
}

fn read_cpu() -> Cpu {
    let info = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let mut model = None;
    let mut vendor = None;
    let mut cores = 0usize;
    for line in info.lines() {
        if line.starts_with("processor") {
            cores += 1;
        } else if model.is_none() && line.starts_with("model name") {
            model = line.splitn(2, ':').nth(1).map(|s| s.trim().to_owned());
        } else if vendor.is_none() && line.starts_with("vendor_id") {
            vendor = line.splitn(2, ':').nth(1).map(|s| s.trim().to_owned());
        }
    }
    Cpu {
        model,
        vendor,
        cores_logical: cores,
    }
}

fn read_acpi_tables(notes: &mut Vec<String>) -> Vec<String> {
    match std::fs::read_dir("/sys/firmware/acpi/tables") {
        Ok(rd) => {
            let mut v: Vec<String> = rd
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        }
        Err(e) => {
            notes.push(format!("ACPI tables unreadable ({e}); re-run as root."));
            Vec::new()
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
