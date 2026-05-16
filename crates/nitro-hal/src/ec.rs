use crate::sysfs::read_trim;
use serde::Serialize;
use std::fs;
use std::path::Path;

const EC_IO: &str = "/sys/kernel/debug/ec/ec0/io";
const WRITE_SUPPORT: &str = "/sys/module/ec_sys/parameters/write_support";

#[derive(Debug, Serialize)]
pub struct EcInfo {
    pub debugfs_present: bool,
    pub io_path: &'static str,
    pub readable: bool,
    pub size: Option<usize>,
    pub write_support: Option<bool>,
    pub dump_hex: Option<String>,
    #[serde(skip)]
    pub dump: Option<Vec<u8>>,
    pub note: Option<String>,
}

pub fn read() -> EcInfo {
    let io = Path::new(EC_IO);
    let debugfs_present = io.exists();
    let write_support = read_trim(WRITE_SUPPORT).map(|v| matches!(v.as_str(), "Y" | "1"));

    let mut info = EcInfo {
        debugfs_present,
        io_path: EC_IO,
        readable: false,
        size: None,
        write_support,
        dump_hex: None,
        dump: None,
        note: None,
    };

    if !debugfs_present {
        info.note = Some("EC debugfs absent. Load read-only with: sudo modprobe ec_sys".into());
        return info;
    }

    match fs::read(io) {
        Ok(bytes) => {
            info.readable = true;
            info.size = Some(bytes.len());
            info.dump_hex = Some(hexdump(&bytes));
            info.dump = Some(bytes);
        }
        Err(e) => info.note = Some(format!("EC present but unreadable ({e}); re-run as root")),
    }
    info
}

pub fn hexdump(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let _ = write!(s, "{:03x}:", i * 16);
        for b in chunk {
            let _ = write!(s, " {b:02x}");
        }
        s.push('\n');
    }
    s
}
