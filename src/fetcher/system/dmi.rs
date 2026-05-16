//! DMI / SMBIOS reads for the `system` fetcher's hardware-identifier kinds.
//!
//! Linux exposes the values as plain files under `/sys/class/dmi/id/`. macOS and Windows would
//! need IOKit / WMI bindings; until those land here, non-Linux callers get `None` and the
//! dispatch renders `"n/a"`.

#[cfg(target_os = "linux")]
const DMI_ROOT: &str = "/sys/class/dmi/id";

pub fn host_vendor() -> Option<String> {
    read_dmi("sys_vendor")
}

pub fn host_model() -> Option<String> {
    read_dmi("product_name")
}

pub fn host_serial() -> Option<String> {
    read_dmi("product_serial")
}

pub fn board_vendor() -> Option<String> {
    read_dmi("board_vendor")
}

pub fn board_model() -> Option<String> {
    read_dmi("board_name")
}

pub fn bios_vendor() -> Option<String> {
    read_dmi("bios_vendor")
}

pub fn bios_version() -> Option<String> {
    read_dmi("bios_version")
}

pub fn bios_date() -> Option<String> {
    read_dmi("bios_date")
}

pub fn chassis() -> Option<String> {
    read_dmi("chassis_type")
        .and_then(|s| s.parse::<u8>().ok())
        .map(chassis_label)
}

#[cfg(target_os = "linux")]
fn read_dmi(name: &str) -> Option<String> {
    std::fs::read_to_string(format!("{DMI_ROOT}/{name}"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(not(target_os = "linux"))]
fn read_dmi(_name: &str) -> Option<String> {
    None
}

/// SMBIOS chassis-type codes (System Management BIOS spec, table 7.4.1).
fn chassis_label(code: u8) -> String {
    match code {
        1 => "Other",
        2 => "Unknown",
        3 => "Desktop",
        4 => "Low Profile Desktop",
        5 => "Pizza Box",
        6 => "Mini Tower",
        7 => "Tower",
        8 => "Portable",
        9 => "Laptop",
        10 => "Notebook",
        11 => "Hand Held",
        12 => "Docking Station",
        13 => "All in One",
        14 => "Sub Notebook",
        15 => "Space-saving",
        16 => "Lunch Box",
        17 => "Main Server Chassis",
        18 => "Expansion Chassis",
        19 => "SubChassis",
        20 => "Bus Expansion Chassis",
        21 => "Peripheral Chassis",
        22 => "RAID Chassis",
        23 => "Rack Mount Chassis",
        24 => "Sealed-case PC",
        25 => "Multi-system",
        26 => "CompactPCI",
        27 => "AdvancedTCA",
        28 => "Blade",
        29 => "Blade Enclosure",
        30 => "Tablet",
        31 => "Convertible",
        32 => "Detachable",
        33 => "IoT Gateway",
        34 => "Embedded PC",
        35 => "Mini PC",
        36 => "Stick PC",
        _ => "Unknown",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chassis_label_covers_known_codes_and_falls_back() {
        assert_eq!(chassis_label(3), "Desktop");
        assert_eq!(chassis_label(9), "Laptop");
        assert_eq!(chassis_label(28), "Blade");
        assert_eq!(chassis_label(99), "Unknown");
    }

    #[test]
    fn chassis_label_maps_every_spec_code() {
        let expected: &[(u8, &str)] = &[
            (1, "Other"),
            (2, "Unknown"),
            (3, "Desktop"),
            (4, "Low Profile Desktop"),
            (5, "Pizza Box"),
            (6, "Mini Tower"),
            (7, "Tower"),
            (8, "Portable"),
            (9, "Laptop"),
            (10, "Notebook"),
            (11, "Hand Held"),
            (12, "Docking Station"),
            (13, "All in One"),
            (14, "Sub Notebook"),
            (15, "Space-saving"),
            (16, "Lunch Box"),
            (17, "Main Server Chassis"),
            (18, "Expansion Chassis"),
            (19, "SubChassis"),
            (20, "Bus Expansion Chassis"),
            (21, "Peripheral Chassis"),
            (22, "RAID Chassis"),
            (23, "Rack Mount Chassis"),
            (24, "Sealed-case PC"),
            (25, "Multi-system"),
            (26, "CompactPCI"),
            (27, "AdvancedTCA"),
            (28, "Blade"),
            (29, "Blade Enclosure"),
            (30, "Tablet"),
            (31, "Convertible"),
            (32, "Detachable"),
            (33, "IoT Gateway"),
            (34, "Embedded PC"),
            (35, "Mini PC"),
            (36, "Stick PC"),
        ];
        expected.iter().for_each(|(code, label)| {
            assert_eq!(chassis_label(*code), *label, "code {code}");
        });
        // Out-of-range codes fall through to the same "Unknown" label as code 2.
        assert_eq!(chassis_label(0), "Unknown");
        assert_eq!(chassis_label(37), "Unknown");
        assert_eq!(chassis_label(u8::MAX), "Unknown");
    }

    #[test]
    fn every_dmi_reader_is_callable_and_returns_an_option() {
        // On Linux the readers may surface real values, but in a sandboxed test
        // environment the files are typically absent — either branch is fine, we
        // only care that the wrappers don't panic and return the documented type.
        let readers: &[fn() -> Option<String>] = &[
            host_vendor,
            host_model,
            host_serial,
            board_vendor,
            board_model,
            bios_vendor,
            bios_version,
            bios_date,
            chassis,
        ];
        readers.iter().for_each(|f| {
            let _ = f();
        });
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_reads_return_none() {
        assert!(host_vendor().is_none());
        assert!(board_model().is_none());
        assert!(chassis().is_none());
    }
}
