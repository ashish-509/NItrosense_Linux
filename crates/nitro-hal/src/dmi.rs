use crate::sysfs::read_trim;
use serde::Serialize;

const DMI: &str = "/sys/class/dmi/id";

#[derive(Debug, Serialize)]
pub struct DmiInfo {
    pub sys_vendor: Option<String>,
    pub product_name: Option<String>,
    pub product_family: Option<String>,
    pub product_version: Option<String>,
    pub board_name: Option<String>,
    pub board_vendor: Option<String>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
    pub chassis_type: Option<String>,
}

pub fn read() -> DmiInfo {
    let f = |n: &str| read_trim(format!("{DMI}/{n}"));
    DmiInfo {
        sys_vendor: f("sys_vendor"),
        product_name: f("product_name"),
        product_family: f("product_family"),
        product_version: f("product_version"),
        board_name: f("board_name"),
        board_vendor: f("board_vendor"),
        bios_vendor: f("bios_vendor"),
        bios_version: f("bios_version"),
        bios_date: f("bios_date"),
        chassis_type: f("chassis_type"),
    }
}
