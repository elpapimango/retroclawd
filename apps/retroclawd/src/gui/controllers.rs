//! The controllers the program knows by name: `assets/controllers/devices.toml`, made from RetroArch's gamepad autoconfig
//! profiles by `tools/convert_autoconfig.py`. A connected pad is looked up by its USB vendor and product id.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

const SHIPPED: &str = include_str!("../../../../assets/controllers/devices.toml");

#[derive(Deserialize)]
struct Devices {
    device: Vec<Device>,
}

#[derive(Deserialize)]
struct Device {
    vid: String,
    pid: String,
    name: String,
}

fn table() -> &'static HashMap<(u16, u16), String> {
    static TABLE: OnceLock<HashMap<(u16, u16), String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let devices: Devices =
            toml::from_str(SHIPPED).expect("the shipped controller list is valid (tested)");
        devices
            .device
            .into_iter()
            .filter_map(|d| {
                let id = |text: &str| u16::from_str_radix(text, 16).ok();
                Some(((id(&d.vid)?, id(&d.pid)?), d.name))
            })
            .collect()
    })
}

/// The name RetroArch's profiles give the controller with this USB id.
pub(crate) fn device_name(vendor_id: u16, product_id: u16) -> Option<&'static str> {
    table().get(&(vendor_id, product_id)).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_list_parses_and_knows_the_8bitdo_ultimate() {
        let devices: Devices = toml::from_str(SHIPPED).unwrap();
        assert!(devices.device.len() > 100, "{}", devices.device.len());
        assert_eq!(
            table().len(),
            devices.device.len(),
            "every entry has a usable id and none repeats"
        );
        assert!(
            device_name(0x2DC8, 0x3106)
                .unwrap()
                .to_lowercase()
                .contains("ultimate")
        );
        assert_eq!(device_name(0xFFFF, 0xFFFF), None);
    }
}
