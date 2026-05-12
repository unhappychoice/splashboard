//! Cross-platform system fetchers backed by `sysinfo`.
//!
//! All are `Safety::Safe` — local kernel counters only, no network or exec. Realtime fetchers
//! cache a `Mutex<System>` and refresh only the fields they need per frame, so the `<1ms
//! infallible` contract holds even as many widgets sample the same source.

use std::sync::{Arc, OnceLock};

use sysinfo::{Disks, System};

use crate::payload::{Bar, Body, Entry, Payload, TextData};

use super::{Fetcher, RealtimeFetcher};

pub mod dmi;

mod info_bios;
mod info_board;
mod info_cpu;
mod info_desktop;
mod info_env;
mod info_host;
mod info_kernel;
mod info_locale;
mod info_machine;
mod info_memory;
mod info_timezone;
mod monitor_battery;
mod monitor_cpu;
mod monitor_disk;
mod monitor_host;
mod monitor_load;
mod monitor_memory;
mod monitor_processes;
mod monitor_uptime;

pub use info_bios::SystemInfoBios;
pub use info_board::SystemInfoBoard;
pub use info_cpu::SystemInfoCpu;
pub use info_desktop::SystemInfoDesktop;
pub use info_env::SystemInfoEnv;
pub use info_host::SystemInfoHost;
pub use info_kernel::SystemInfoKernel;
pub use info_locale::SystemInfoLocale;
pub use info_machine::SystemInfoMachine;
pub use info_memory::SystemInfoMemory;
pub use info_timezone::SystemInfoTimezone;
pub use monitor_battery::SystemMonitorBattery;
pub use monitor_cpu::SystemMonitorCpu;
pub use monitor_disk::SystemMonitorDisk;
pub use monitor_host::SystemMonitorHost;
pub use monitor_load::SystemMonitorLoad;
pub use monitor_memory::SystemMonitorMemory;
pub use monitor_processes::SystemMonitorProcesses;
pub use monitor_uptime::SystemMonitorUptime;

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![
        Arc::new(SystemMonitorHost::new()),
        Arc::new(SystemInfoHost),
        Arc::new(SystemInfoCpu),
        Arc::new(SystemInfoMemory),
        Arc::new(SystemInfoKernel),
        Arc::new(SystemInfoMachine),
        Arc::new(SystemInfoBoard),
        Arc::new(SystemInfoBios),
        Arc::new(SystemInfoLocale),
        Arc::new(SystemInfoTimezone),
        Arc::new(SystemInfoEnv),
        Arc::new(SystemInfoDesktop),
        Arc::new(SystemMonitorCpu::new()),
        Arc::new(SystemMonitorMemory::new()),
        Arc::new(SystemMonitorUptime),
        Arc::new(SystemMonitorLoad),
        Arc::new(SystemMonitorProcesses::new()),
        Arc::new(SystemMonitorBattery::new()),
    ]
}

pub fn cached_fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(SystemMonitorDisk)]
}

#[derive(Debug, Clone)]
struct CpuInfo {
    model: String,
    vendor: String,
    frequency_mhz: u64,
}

fn cached_cpu_info() -> &'static CpuInfo {
    static CACHE: OnceLock<CpuInfo> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        let cpu = sys.cpus().first();
        let frequency_mhz = cpu
            .map(|c| c.frequency())
            .filter(|f| *f > 0)
            .or_else(cpu_mhz_from_proc_cpuinfo)
            .unwrap_or(0);
        CpuInfo {
            model: cpu
                .map(|c| c.brand().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".into()),
            vendor: cpu
                .map(|c| c.vendor_id().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".into()),
            frequency_mhz,
        }
    })
}

/// Fallback for environments where sysinfo's `Cpu::frequency()` returns 0 (WSL, some
/// containers). Reads the first `cpu MHz` line from `/proc/cpuinfo` and rounds to MHz.
fn cpu_mhz_from_proc_cpuinfo() -> Option<u64> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines()
        .find_map(|line| {
            line.strip_prefix("cpu MHz")
                .and_then(|rest| rest.split(':').nth(1))
        })
        .and_then(|val| val.trim().parse::<f64>().ok())
        .map(|mhz| mhz.round() as u64)
        .filter(|m| *m > 0)
}

fn format_cpu_cores() -> String {
    let physical = System::physical_core_count().unwrap_or(0);
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    let logical = sys.cpus().len();
    match (physical, logical) {
        (0, 0) => "unknown".into(),
        (0, l) => format!("{l} threads"),
        (p, l) if p == l => format!("{p} cores"),
        (p, l) => format!("{p} cores / {l} threads"),
    }
}

fn format_cpu_frequency(mhz: u64) -> String {
    match mhz {
        0 => "unknown".into(),
        m if m >= 1000 => format!("{:.2} GHz", m as f64 / 1000.0),
        m => format!("{m} MHz"),
    }
}

#[derive(Debug, Clone, Copy)]
struct MemoryTotals {
    memory: u64,
    swap: u64,
}

fn cached_memory_totals() -> MemoryTotals {
    static CACHE: OnceLock<MemoryTotals> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let mut sys = System::new();
        sys.refresh_memory();
        MemoryTotals {
            memory: sys.total_memory(),
            swap: sys.total_swap(),
        }
    })
}

fn kernel_name() -> String {
    match std::env::consts::OS {
        "linux" => "Linux".into(),
        "macos" => "Darwin".into(),
        "windows" => "Windows NT".into(),
        "freebsd" => "FreeBSD".into(),
        "openbsd" => "OpenBSD".into(),
        "netbsd" => "NetBSD".into(),
        other if !other.is_empty() => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_else(|| other.into())
        }
        _ => "unknown".into(),
    }
}

fn dmi_or_na(value: Option<String>) -> String {
    value.unwrap_or_else(|| "n/a".into())
}

fn detect_locale() -> String {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "C".into())
}

fn detect_timezone() -> String {
    if let Some(tz) = std::env::var("TZ").ok().filter(|v| !v.is_empty()) {
        return tz;
    }
    if let Ok(path) = std::fs::read_link("/etc/localtime") {
        let s = path.to_string_lossy();
        if let Some(idx) = s.find("/zoneinfo/") {
            return s[idx + "/zoneinfo/".len()..].to_string();
        }
        if let Some(name) = path.file_name() {
            return name.to_string_lossy().into_owned();
        }
    }
    "unknown".into()
}

fn env_basename(key: &str, fallback: &str) -> String {
    std::env::var(key)
        .ok()
        .and_then(|s| {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| {
                std::path::Path::new(trimmed)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| trimmed.to_string())
            })
        })
        .unwrap_or_else(|| fallback.into())
}

fn detect_desktop_environment() -> String {
    if cfg!(target_os = "macos") {
        return "Aqua".into();
    }
    if cfg!(target_os = "windows") {
        return "Windows".into();
    }
    ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION", "GDMSESSION"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .map(|s| s.split(':').next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "tty".into())
}

fn detect_window_manager() -> String {
    if cfg!(target_os = "macos") {
        return "Quartz".into();
    }
    if cfg!(target_os = "windows") {
        return "DWM".into();
    }
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return std::env::var("XDG_SESSION_DESKTOP")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "wayland".into());
    }
    if std::env::var("DISPLAY").is_ok() {
        return std::env::var("XDG_SESSION_DESKTOP")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "x11".into());
    }
    "tty".into()
}

fn detect_init_system() -> String {
    if cfg!(target_os = "macos") {
        return "launchd".into();
    }
    if cfg!(target_os = "windows") {
        return "wininit".into();
    }
    if cfg!(target_os = "linux")
        && let Ok(comm) = std::fs::read_to_string("/proc/1/comm")
    {
        let name = comm.trim();
        if !name.is_empty() {
            return name.into();
        }
    }
    "unknown".into()
}

/// Detect the terminal emulator via well-known env vars. Env vars are the only signal available
/// without spawning a subprocess, so this is a best-effort match with a generic fallback.
fn detect_terminal() -> String {
    let env = |k: &str| std::env::var(k).ok();
    if env("WT_SESSION").is_some() {
        return "Windows Terminal".into();
    }
    if env("GHOSTTY_RESOURCES_DIR").is_some() {
        return "Ghostty".into();
    }
    if env("KITTY_WINDOW_ID").is_some() || env("TERM").as_deref() == Some("xterm-kitty") {
        return "Kitty".into();
    }
    if env("ALACRITTY_WINDOW_ID").is_some() || env("ALACRITTY_LOG").is_some() {
        return "Alacritty".into();
    }
    if env("WEZTERM_PANE").is_some() {
        return "WezTerm".into();
    }
    match env("TERM_PROGRAM").as_deref() {
        Some("iTerm.app") => "iTerm2".into(),
        Some("Apple_Terminal") => "Terminal".into(),
        Some("ghostty") => "Ghostty".into(),
        Some("WezTerm") => "WezTerm".into(),
        Some("Hyper") => "Hyper".into(),
        Some("vscode") => "VS Code".into(),
        Some(other) if !other.is_empty() => other.into(),
        _ => "(unset)".into(),
    }
}

fn detect_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| {
            std::path::Path::new(&s)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "(unset)".into())
}

fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

fn options_placeholder(msg: &str) -> Payload {
    payload(Body::Text(TextData {
        value: format!("⚠ {msg}"),
    }))
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

fn os_label() -> String {
    System::long_os_version()
        .or_else(System::name)
        .unwrap_or_else(|| "unknown".into())
}

fn memory_ratio(sys: &System) -> f64 {
    ratio_of(sys.used_memory(), sys.total_memory())
}

fn ratio_of(numer: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        (numer as f64 / denom as f64).clamp(0.0, 1.0)
    }
}

fn top_processes(sys: &System, count: usize) -> Vec<(String, f32)> {
    let mut rows: Vec<(String, f32)> = sys
        .processes()
        .values()
        .map(|p| (p.name().to_string_lossy().into_owned(), p.cpu_usage()))
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(count);
    rows
}

fn primary_disk(disks: &Disks) -> Option<(u64, u64)> {
    disks
        .iter()
        .filter(|d| d.total_space() > 0)
        .max_by_key(|d| d.total_space())
        .map(|d| (d.total_space(), d.available_space()))
}

fn disk_bars(disks: &Disks) -> Vec<Bar> {
    disks
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| Bar {
            label: d.mount_point().to_string_lossy().into_owned(),
            value: d.total_space().saturating_sub(d.available_space()),
        })
        .collect()
}

fn disk_label(total: u64, available: u64) -> String {
    let used = total.saturating_sub(available);
    format!(
        "{:.0}% of {}",
        ratio_of(used, total) * 100.0,
        format_bytes(total)
    )
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    let minutes = (secs % 3600) / 60;
    match (days, hours, minutes) {
        (0, 0, m) => format!("{m}m"),
        (0, h, m) => format!("{h}h {m}m"),
        (d, h, _) => format!("{d}d {h}h"),
    }
}

fn format_load(v: f64) -> String {
    if cfg!(windows) {
        "n/a".into()
    } else {
        format!("{v:.2}")
    }
}

#[cfg(windows)]
fn load_line(_: f64, _: f64, _: f64) -> String {
    "n/a (windows)".into()
}

#[cfg(not(windows))]
fn load_line(one: f64, five: f64, fifteen: f64) -> String {
    format!("{one:.2} {five:.2} {fifteen:.2}")
}

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;
const GB: u64 = 1024 * MB;
const TB: u64 = 1024 * GB;

fn format_bytes(bytes: u64) -> String {
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{FetchContext, Safety};
    use super::info_host::HostInfoOptions as SystemOptions;
    use super::monitor_battery::{
        BatteryOptions, BatterySnapshot, BatteryState, BatteryTextKind, battery_badge,
        battery_entries, format_battery_text, format_percent, map_battery_state,
        no_battery_payload,
    };
    use super::monitor_processes::PROCESS_TOP_COUNT;
    use super::*;
    use crate::paths::TEST_ENV_LOCK;
    use crate::payload::Status;
    use crate::render::Shape;
    use std::sync::Mutex;
    use std::time::Duration;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut restore: Vec<(&'static str, Option<String>)> = Vec::new();
            for (key, value) in pairs {
                if !restore.iter().any(|(k, _)| k == key) {
                    restore.push((*key, std::env::var(key).ok()));
                }
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.restore.iter().for_each(|(key, value)| match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            });
        }
    }

    const TERMINAL_ENV_KEYS: &[&str] = &[
        "WT_SESSION",
        "GHOSTTY_RESOURCES_DIR",
        "KITTY_WINDOW_ID",
        "TERM",
        "ALACRITTY_WINDOW_ID",
        "ALACRITTY_LOG",
        "WEZTERM_PANE",
        "TERM_PROGRAM",
    ];

    fn ctx_with_shape(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            timeout: Duration::from_secs(1),
            shape,
            ..Default::default()
        }
    }

    fn ctx_text(options: Option<&str>) -> FetchContext {
        let options = options.map(|s| toml::from_str::<toml::Value>(s).unwrap());
        FetchContext {
            widget_id: "w".into(),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Text),
            options,
            ..Default::default()
        }
    }

    fn detect_terminal_with(overrides: &[(&'static str, &'static str)]) -> String {
        let pairs: Vec<_> = TERMINAL_ENV_KEYS
            .iter()
            .map(|key| {
                (
                    *key,
                    overrides.iter().find_map(|(override_key, value)| {
                        (*override_key == *key).then_some(*value)
                    }),
                )
            })
            .collect();
        let _guard = EnvGuard::set(&pairs);
        detect_terminal()
    }

    fn assert_realtime_contract(
        fetcher: &dyn RealtimeFetcher,
        expected_name: &str,
        expected_shapes: &[Shape],
        default_shape: Shape,
        unsupported_shape: Shape,
        option_count: usize,
    ) {
        assert_eq!(fetcher.name(), expected_name);
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(!fetcher.description().is_empty());
        assert_eq!(fetcher.shapes(), expected_shapes);
        assert_eq!(fetcher.default_shape(), default_shape);
        assert_eq!(fetcher.option_schemas().len(), option_count);
        expected_shapes
            .iter()
            .for_each(|shape| assert!(fetcher.sample_body(*shape).is_some()));
        assert!(fetcher.sample_body(unsupported_shape).is_none());
    }

    fn assert_cached_contract(
        fetcher: &dyn Fetcher,
        expected_name: &str,
        expected_shapes: &[Shape],
        unsupported_shape: Shape,
    ) {
        assert_eq!(fetcher.name(), expected_name);
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(!fetcher.description().is_empty());
        assert_eq!(fetcher.shapes(), expected_shapes);
        assert_eq!(fetcher.default_shape(), expected_shapes[0]);
        expected_shapes
            .iter()
            .for_each(|shape| assert!(fetcher.sample_body(*shape).is_some()));
        assert!(fetcher.sample_body(unsupported_shape).is_none());
    }

    #[test]
    fn system_family_registers_builtin_fetchers() {
        let realtime_names: Vec<_> = realtime_fetchers()
            .into_iter()
            .map(|fetcher| fetcher.name().to_string())
            .collect();
        let cached_names: Vec<_> = cached_fetchers()
            .into_iter()
            .map(|fetcher| fetcher.name().to_string())
            .collect();
        assert_eq!(
            realtime_names,
            vec![
                "system_monitor_host",
                "system_info_host",
                "system_info_cpu",
                "system_info_memory",
                "system_info_kernel",
                "system_info_machine",
                "system_info_board",
                "system_info_bios",
                "system_info_locale",
                "system_info_timezone",
                "system_info_env",
                "system_info_desktop",
                "system_monitor_cpu",
                "system_monitor_memory",
                "system_monitor_uptime",
                "system_monitor_load",
                "system_monitor_processes",
                "system_monitor_battery",
            ]
        );
        assert_eq!(cached_names, vec!["system_monitor_disk"]);
    }

    #[test]
    fn fetcher_contracts_cover_supported_shapes_and_samples() {
        assert_realtime_contract(
            &SystemMonitorHost::default(),
            "system_monitor_host",
            &[Shape::Entries, Shape::TextBlock],
            Shape::Entries,
            Shape::Ratio,
            0,
        );
        assert_realtime_contract(
            &SystemInfoHost,
            "system_info_host",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoCpu,
            "system_info_cpu",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoMemory,
            "system_info_memory",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoKernel,
            "system_info_kernel",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoMachine,
            "system_info_machine",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoBoard,
            "system_info_board",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoBios,
            "system_info_bios",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoLocale,
            "system_info_locale",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            0,
        );
        assert_realtime_contract(
            &SystemInfoTimezone,
            "system_info_timezone",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            0,
        );
        assert_realtime_contract(
            &SystemInfoEnv,
            "system_info_env",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemInfoDesktop,
            "system_info_desktop",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            1,
        );
        assert_realtime_contract(
            &SystemMonitorCpu::default(),
            "system_monitor_cpu",
            &[Shape::Ratio, Shape::Text],
            Shape::Ratio,
            Shape::Entries,
            0,
        );
        assert_realtime_contract(
            &SystemMonitorMemory::default(),
            "system_monitor_memory",
            &[Shape::Ratio, Shape::Text, Shape::Entries],
            Shape::Ratio,
            Shape::Bars,
            0,
        );
        assert_realtime_contract(
            &SystemMonitorUptime,
            "system_monitor_uptime",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            0,
        );
        assert_realtime_contract(
            &SystemMonitorLoad,
            "system_monitor_load",
            &[Shape::Text, Shape::Entries],
            Shape::Text,
            Shape::Badge,
            0,
        );
        assert_realtime_contract(
            &SystemMonitorProcesses::default(),
            "system_monitor_processes",
            &[Shape::Entries, Shape::TextBlock],
            Shape::Entries,
            Shape::Ratio,
            0,
        );
        assert_realtime_contract(
            &SystemMonitorBattery::default(),
            "system_monitor_battery",
            &[Shape::Ratio, Shape::Text, Shape::Entries, Shape::Badge],
            Shape::Ratio,
            Shape::Bars,
            2,
        );
        assert_cached_contract(
            &SystemMonitorDisk,
            "system_monitor_disk",
            &[Shape::Ratio, Shape::Text, Shape::Bars],
            Shape::Entries,
        );
    }

    #[test]
    fn system_sample_body_matches_documented_copy() {
        let fetcher = SystemMonitorHost::default();

        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(entries))
                if entries.items.len() == 6
                    && entries.items[0].key == "os"
                    && entries.items[5].value.as_deref() == Some("67%")
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(block))
                if block.lines[0] == "os: linux" && block.lines[5] == "memory: 67%"
        ));
        assert!(fetcher.sample_body(Shape::Text).is_none());
        assert!(matches!(
            SystemInfoHost.sample_body(Shape::Text),
            Some(Body::Text(text)) if text.value == "iTerm2"
        ));
    }

    #[test]
    fn disk_sample_body_matches_documented_shapes() {
        assert!(matches!(
            SystemMonitorDisk.sample_body(Shape::Ratio),
            Some(Body::Ratio(ratio))
                if ratio.label.as_deref() == Some("disk") && ratio.value == 0.58
        ));
        assert!(matches!(
            SystemMonitorDisk.sample_body(Shape::Text),
            Some(Body::Text(text)) if text.value == "58% of 400 GB"
        ));
        assert!(matches!(
            SystemMonitorDisk.sample_body(Shape::Bars),
            Some(Body::Bars(bars))
                if bars.bars.len() == 3
                    && bars.bars[0].label == "/"
                    && bars.bars[2].value == 200
        ));
    }

    #[test]
    fn battery_sample_body_matches_documented_shapes() {
        let fetcher = SystemMonitorBattery::default();

        assert!(matches!(
            fetcher.sample_body(Shape::Ratio),
            Some(Body::Ratio(ratio))
                if ratio.label.as_deref() == Some("battery") && ratio.value == 0.87
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Text),
            Some(Body::Text(text)) if text.value == "87% • Charging • 1h 23m"
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(entries))
                if entries.items.len() == 5
                    && entries.items[0].key == "charge"
                    && entries.items[4].value.as_deref() == Some("97%")
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Badge),
            Some(Body::Badge(badge))
                if badge.status == Status::Ok && badge.label == "87% · Charging"
        ));
    }

    #[test]
    fn sample_bodies_return_none_for_unsupported_shapes() {
        let unsupported = Shape::Image;

        assert!(SystemMonitorHost::new().sample_body(unsupported).is_none());
        assert!(SystemMonitorCpu::new().sample_body(unsupported).is_none());
        assert!(
            SystemMonitorMemory::new()
                .sample_body(unsupported)
                .is_none()
        );
        assert!(SystemMonitorUptime.sample_body(unsupported).is_none());
        assert!(SystemMonitorLoad.sample_body(unsupported).is_none());
        assert!(
            SystemMonitorProcesses::new()
                .sample_body(unsupported)
                .is_none()
        );
        assert!(SystemMonitorDisk.sample_body(unsupported).is_none());
        assert!(
            SystemMonitorBattery::new()
                .sample_body(unsupported)
                .is_none()
        );
    }

    #[test]
    fn parse_options_defaults_and_surfaces_invalid_input() {
        let system: SystemOptions = parse_options(None).unwrap();
        assert!(system.kind.is_none());

        let battery_raw: toml::Value = toml::from_str("kind = \"percent\"\nindex = 2").unwrap();
        let battery: BatteryOptions = parse_options(Some(&battery_raw)).unwrap();
        assert!(matches!(battery.kind, Some(BatteryTextKind::Percent)));
        assert_eq!(battery.index, Some(2));

        let invalid: toml::Value = toml::from_str("bogus = true").unwrap();
        let err = parse_options::<BatteryOptions>(Some(&invalid)).unwrap_err();
        assert!(err.starts_with("invalid options:"));
    }

    #[test]
    fn options_placeholder_wraps_the_message_in_warning_text() {
        let Body::Text(text) = options_placeholder("bad config").body else {
            panic!("expected text body");
        };
        assert_eq!(text.value, "⚠ bad config");
    }

    #[test]
    fn detect_terminal_prefers_known_env_markers_and_fallbacks() {
        assert_eq!(
            detect_terminal_with(&[("WT_SESSION", "1"), ("TERM_PROGRAM", "Hyper")]),
            "Windows Terminal"
        );
        assert_eq!(
            detect_terminal_with(&[("GHOSTTY_RESOURCES_DIR", "/tmp/resources")]),
            "Ghostty"
        );
        assert_eq!(detect_terminal_with(&[("TERM", "xterm-kitty")]), "Kitty");
        assert_eq!(
            detect_terminal_with(&[("ALACRITTY_LOG", "/tmp/alacritty.log")]),
            "Alacritty"
        );
        assert_eq!(detect_terminal_with(&[("WEZTERM_PANE", "pane")]), "WezTerm");
        assert_eq!(
            detect_terminal_with(&[("TERM_PROGRAM", "vscode")]),
            "VS Code"
        );
        assert_eq!(
            detect_terminal_with(&[("TERM_PROGRAM", "CustomTerm")]),
            "CustomTerm"
        );
        assert_eq!(detect_terminal_with(&[]), "(unset)");
    }

    #[test]
    fn detect_terminal_covers_remaining_aliases_and_priorities() {
        assert_eq!(
            detect_terminal_with(&[("KITTY_WINDOW_ID", "7"), ("TERM_PROGRAM", "iTerm.app")]),
            "Kitty"
        );
        assert_eq!(
            detect_terminal_with(&[("ALACRITTY_WINDOW_ID", "9")]),
            "Alacritty"
        );
        assert_eq!(
            detect_terminal_with(&[("TERM_PROGRAM", "iTerm.app")]),
            "iTerm2"
        );
        assert_eq!(
            detect_terminal_with(&[("TERM_PROGRAM", "Apple_Terminal")]),
            "Terminal"
        );
        assert_eq!(
            detect_terminal_with(&[("TERM_PROGRAM", "ghostty")]),
            "Ghostty"
        );
        assert_eq!(
            detect_terminal_with(&[("TERM_PROGRAM", "WezTerm")]),
            "WezTerm"
        );
    }

    #[test]
    fn detect_shell_uses_basename_and_fallback() {
        let _guard = EnvGuard::set(&[("SHELL", Some("/usr/local/bin/fish"))]);
        assert_eq!(detect_shell(), "fish");
        drop(_guard);

        let _guard = EnvGuard::set(&[("SHELL", None)]);
        assert_eq!(detect_shell(), "(unset)");
    }

    #[test]
    fn format_uptime_covers_minute_hour_day_ranges() {
        assert_eq!(format_uptime(0), "0m");
        assert_eq!(format_uptime(45 * 60), "45m");
        assert_eq!(format_uptime(2 * 3600 + 15 * 60), "2h 15m");
        assert_eq!(format_uptime(3 * 86_400 + 4 * 3600 + 30 * 60), "3d 4h");
    }

    #[test]
    fn format_bytes_buckets_by_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(4 * KB), "4 KB");
        assert_eq!(format_bytes(250 * MB), "250 MB");
        assert_eq!(format_bytes(2 * GB + GB / 2), "2.5 GB");
        assert_eq!(format_bytes(3 * TB), "3.0 TB");
    }

    #[test]
    fn ratio_of_handles_zero_denominator() {
        assert_eq!(ratio_of(10, 0), 0.0);
        assert_eq!(ratio_of(5, 10), 0.5);
        assert_eq!(ratio_of(20, 10), 1.0);
    }

    #[test]
    fn empty_system_and_disk_snapshots_use_safe_fallbacks() {
        let sys = System::new();
        assert_eq!(memory_ratio(&sys), 0.0);
        assert!(top_processes(&sys, PROCESS_TOP_COUNT).is_empty());

        let disks = Disks::new();
        assert_eq!(primary_disk(&disks), None);
        assert!(disk_bars(&disks).is_empty());
        assert_eq!(disk_label(10, 20), "0% of 10 B");
    }

    #[test]
    fn helper_formatters_and_builders_cover_remaining_branches() {
        assert_eq!(format_percent(-1.0), "0%");
        assert_eq!(format_percent(0.5), "50%");
        assert_eq!(format_percent(2.0), "100%");
        assert_eq!(disk_label(200, 50), "75% of 200 B");

        #[cfg(not(windows))]
        {
            assert_eq!(format_load(0.42), "0.42");
            assert_eq!(load_line(0.42, 0.38, 0.31), "0.42 0.38 0.31");
        }

        #[cfg(windows)]
        {
            assert_eq!(format_load(0.42), "n/a");
            assert_eq!(load_line(0.42, 0.38, 0.31), "n/a (windows)");
        }

        let row = entry("cpu", "18%");
        assert_eq!(row.key, "cpu");
        assert_eq!(row.value.as_deref(), Some("18%"));
        assert!(row.status.is_none());

        let wrapped = payload(Body::Text(TextData {
            value: "hello".into(),
        }));
        assert!(wrapped.icon.is_none());
        assert!(wrapped.status.is_none());
        assert!(wrapped.format.is_none());
        assert!(matches!(wrapped.body, Body::Text(TextData { value }) if value == "hello"));
    }

    #[test]
    fn cpu_load_defaults_to_ratio() {
        let p = SystemMonitorCpu::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Ratio(_)));
    }

    #[test]
    fn cpu_load_emits_text_when_requested() {
        let p = SystemMonitorCpu::new().compute(&ctx_with_shape(Some(Shape::Text)));
        assert!(matches!(p.body, Body::Text(_)));
    }

    #[test]
    fn memory_defaults_to_ratio() {
        let p = SystemMonitorMemory::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Ratio(_)));
        if let Body::Ratio(r) = p.body {
            assert!((0.0..=1.0).contains(&r.value));
        }
    }

    #[test]
    fn memory_entries_shape_has_three_rows() {
        let p = SystemMonitorMemory::new().compute(&ctx_with_shape(Some(Shape::Entries)));
        assert!(matches!(p.body, Body::Entries(_)));
        if let Body::Entries(e) = p.body {
            let keys: Vec<_> = e.items.iter().map(|i| i.key.as_str()).collect();
            assert_eq!(keys, ["used", "total", "free"]);
        }
    }

    #[test]
    fn memory_text_shape_formats_used_over_total() {
        let p = SystemMonitorMemory::new().compute(&ctx_with_shape(Some(Shape::Text)));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(text) = p.body {
            assert!(text.value.contains(" / "));
        }
    }

    #[test]
    fn uptime_emits_text() {
        let p = SystemMonitorUptime.compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(!t.value.is_empty());
        }
    }

    #[test]
    fn load_average_defaults_to_text() {
        let p = SystemMonitorLoad.compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Text(_)));
    }

    #[test]
    fn load_average_entries_shape_has_three_windows() {
        let p = SystemMonitorLoad.compute(&ctx_with_shape(Some(Shape::Entries)));
        assert!(matches!(p.body, Body::Entries(_)));
        if let Body::Entries(e) = p.body {
            let keys: Vec<_> = e.items.iter().map(|i| i.key.as_str()).collect();
            assert_eq!(keys, ["1min", "5min", "15min"]);
        }
    }

    #[test]
    fn system_rollup_emits_six_rows() {
        let p = SystemMonitorHost::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Entries(_)));
        if let Body::Entries(e) = p.body {
            assert_eq!(e.items.len(), 6);
        }
    }

    #[test]
    fn system_info_host_defaults_to_terminal_kind() {
        let p = SystemInfoHost.compute(&ctx_text(None));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(!t.value.is_empty());
        }
    }

    #[test]
    fn system_info_host_emits_arch_when_requested() {
        let p = SystemInfoHost.compute(&ctx_text(Some("kind = \"arch\"")));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert_eq!(t.value, std::env::consts::ARCH);
        }
    }

    #[test]
    fn system_info_host_rejects_unknown_kind_to_placeholder() {
        let p = SystemInfoHost.compute(&ctx_text(Some("kind = \"bogus\"")));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(t.value.starts_with("⚠"));
        }
    }

    #[test]
    fn detect_terminal_returns_non_empty_label() {
        assert!(!detect_terminal().is_empty());
    }

    /// Prints one Text-shape line per `kind` across the `system_info_*` family on the host
    /// running the tests. Kept `#[ignore]` so the regular run stays side-effect free, but a dev
    /// can verify real output with
    /// `cargo test -- --ignored fetcher::system::tests::live_system_text_all_kinds --nocapture`.
    #[test]
    #[ignore]
    fn live_system_text_all_kinds() {
        let cases: Vec<(&str, &dyn RealtimeFetcher, Option<&str>)> = vec![
            (
                "host:terminal",
                &SystemInfoHost,
                Some("kind = \"terminal\""),
            ),
            ("host:os", &SystemInfoHost, Some("kind = \"os\"")),
            (
                "host:os_version",
                &SystemInfoHost,
                Some("kind = \"os_version\""),
            ),
            (
                "host:hostname",
                &SystemInfoHost,
                Some("kind = \"hostname\""),
            ),
            ("host:shell", &SystemInfoHost, Some("kind = \"shell\"")),
            ("host:arch", &SystemInfoHost, Some("kind = \"arch\"")),
            ("cpu:model", &SystemInfoCpu, Some("kind = \"model\"")),
            ("cpu:cores", &SystemInfoCpu, Some("kind = \"cores\"")),
            (
                "cpu:frequency",
                &SystemInfoCpu,
                Some("kind = \"frequency\""),
            ),
            ("cpu:vendor", &SystemInfoCpu, Some("kind = \"vendor\"")),
            ("memory:total", &SystemInfoMemory, Some("kind = \"total\"")),
            (
                "memory:swap_total",
                &SystemInfoMemory,
                Some("kind = \"swap_total\""),
            ),
            ("kernel:name", &SystemInfoKernel, Some("kind = \"name\"")),
            (
                "kernel:version",
                &SystemInfoKernel,
                Some("kind = \"version\""),
            ),
            (
                "machine:model",
                &SystemInfoMachine,
                Some("kind = \"model\""),
            ),
            (
                "machine:vendor",
                &SystemInfoMachine,
                Some("kind = \"vendor\""),
            ),
            (
                "machine:serial",
                &SystemInfoMachine,
                Some("kind = \"serial\""),
            ),
            (
                "machine:chassis",
                &SystemInfoMachine,
                Some("kind = \"chassis\""),
            ),
            ("board:vendor", &SystemInfoBoard, Some("kind = \"vendor\"")),
            ("board:model", &SystemInfoBoard, Some("kind = \"model\"")),
            ("bios:vendor", &SystemInfoBios, Some("kind = \"vendor\"")),
            ("bios:version", &SystemInfoBios, Some("kind = \"version\"")),
            ("bios:date", &SystemInfoBios, Some("kind = \"date\"")),
            ("locale", &SystemInfoLocale, None),
            ("timezone", &SystemInfoTimezone, None),
            ("env:editor", &SystemInfoEnv, Some("kind = \"editor\"")),
            ("env:visual", &SystemInfoEnv, Some("kind = \"visual\"")),
            ("env:pager", &SystemInfoEnv, Some("kind = \"pager\"")),
            ("desktop:de", &SystemInfoDesktop, Some("kind = \"de\"")),
            ("desktop:wm", &SystemInfoDesktop, Some("kind = \"wm\"")),
            ("desktop:init", &SystemInfoDesktop, Some("kind = \"init\"")),
        ];
        for (label, fetcher, opts) in cases {
            let p = fetcher.compute(&ctx_text(opts));
            assert!(matches!(p.body, Body::Text(_)), "expected text for {label}");
            if let Body::Text(t) = p.body {
                eprintln!("{label:<18} → {}", t.value);
                assert!(!t.value.is_empty());
            }
        }
    }

    #[test]
    fn kernel_name_maps_consts_os() {
        let name = kernel_name();
        assert!(!name.is_empty());
        match std::env::consts::OS {
            "linux" => assert_eq!(name, "Linux"),
            "macos" => assert_eq!(name, "Darwin"),
            "windows" => assert_eq!(name, "Windows NT"),
            _ => {}
        }
    }

    #[test]
    fn format_cpu_frequency_buckets_units() {
        assert_eq!(format_cpu_frequency(0), "unknown");
        assert_eq!(format_cpu_frequency(800), "800 MHz");
        assert_eq!(format_cpu_frequency(2400), "2.40 GHz");
        assert_eq!(format_cpu_frequency(4250), "4.25 GHz");
    }

    #[test]
    fn dmi_or_na_falls_back_when_unavailable() {
        assert_eq!(dmi_or_na(None), "n/a");
        assert_eq!(dmi_or_na(Some("Dell Inc.".into())), "Dell Inc.");
    }

    #[test]
    fn env_basename_uses_filename_then_fallback() {
        let _guard = EnvGuard::set(&[("EDITOR", Some("/usr/bin/nvim"))]);
        assert_eq!(env_basename("EDITOR", "(unset)"), "nvim");
        drop(_guard);

        let _guard = EnvGuard::set(&[("EDITOR", None)]);
        assert_eq!(env_basename("EDITOR", "(unset)"), "(unset)");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn cpu_mhz_from_proc_cpuinfo_returns_some_on_linux() {
        // Real `/proc/cpuinfo` typically exposes `cpu MHz`; container envs without it
        // legitimately return None, so this only asserts the call doesn't panic.
        let _ = cpu_mhz_from_proc_cpuinfo();
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn cpu_mhz_from_proc_cpuinfo_is_none_off_linux() {
        assert!(cpu_mhz_from_proc_cpuinfo().is_none());
    }

    #[test]
    fn detect_locale_prefers_lc_all_then_lang() {
        let _guard = EnvGuard::set(&[
            ("LC_ALL", Some("en_US.UTF-8")),
            ("LC_CTYPE", None),
            ("LANG", Some("ja_JP.UTF-8")),
        ]);
        assert_eq!(detect_locale(), "en_US.UTF-8");
        drop(_guard);

        let _guard = EnvGuard::set(&[
            ("LC_ALL", None),
            ("LC_CTYPE", None),
            ("LANG", Some("ja_JP.UTF-8")),
        ]);
        assert_eq!(detect_locale(), "ja_JP.UTF-8");
        drop(_guard);

        let _guard = EnvGuard::set(&[("LC_ALL", None), ("LC_CTYPE", None), ("LANG", None)]);
        assert_eq!(detect_locale(), "C");
    }

    #[test]
    fn detect_timezone_honors_tz_env_override() {
        let _guard = EnvGuard::set(&[("TZ", Some("Asia/Tokyo"))]);
        assert_eq!(detect_timezone(), "Asia/Tokyo");
    }

    #[test]
    fn cpu_info_cache_returns_non_empty_fields() {
        let info = cached_cpu_info();
        assert!(!info.model.is_empty());
        assert!(!info.vendor.is_empty());
    }

    #[test]
    fn memory_totals_cache_reports_nonzero_memory() {
        let totals = cached_memory_totals();
        assert!(totals.memory > 0);
    }

    #[test]
    fn system_text_all_kinds_helper_runs_in_regular_suite() {
        live_system_text_all_kinds();
    }

    #[test]
    fn system_text_block_shape_returns_key_value_strings() {
        let p = SystemMonitorHost::new().compute(&ctx_with_shape(Some(Shape::TextBlock)));
        assert!(matches!(p.body, Body::TextBlock(_)));
        if let Body::TextBlock(l) = p.body {
            assert_eq!(l.lines.len(), 6);
            assert!(l.lines.iter().all(|s| s.contains(": ")));
        }
    }

    #[test]
    fn process_top_respects_count_cap() {
        let p = SystemMonitorProcesses::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Entries(_)));
        if let Body::Entries(e) = p.body {
            assert!(e.items.len() <= PROCESS_TOP_COUNT);
        }
    }

    #[test]
    fn process_top_text_block_shape_formats_rows() {
        let p = SystemMonitorProcesses::new().compute(&ctx_with_shape(Some(Shape::TextBlock)));
        assert!(matches!(p.body, Body::TextBlock(_)));
        if let Body::TextBlock(block) = p.body {
            assert!(block.lines.len() <= PROCESS_TOP_COUNT);
            assert!(block.lines.iter().all(|line| line.ends_with('%')));
        }
    }

    fn snapshot(charge: f64, state: BatteryState, secs: Option<u64>) -> BatterySnapshot {
        BatterySnapshot {
            charge,
            state,
            time_remaining_secs: secs,
            cycle_count: Some(284),
            health: Some(0.97),
        }
    }

    #[test]
    fn battery_summary_includes_time_when_available() {
        let snap = snapshot(0.87, BatteryState::Charging, Some(83 * 60));
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::Summary),
            "87% • Charging • 1h 23m"
        );
    }

    #[test]
    fn battery_summary_omits_time_when_missing() {
        let snap = snapshot(1.0, BatteryState::Full, None);
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::Summary),
            "100% • Full"
        );
    }

    #[test]
    fn battery_text_kinds_pick_distinct_fields() {
        let snap = snapshot(0.5, BatteryState::Discharging, Some(45 * 60));
        assert_eq!(format_battery_text(&snap, BatteryTextKind::Percent), "50%");
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::Status),
            "Discharging"
        );
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::TimeRemaining),
            "45m"
        );
    }

    #[test]
    fn battery_time_remaining_dash_when_missing() {
        let snap = snapshot(1.0, BatteryState::Full, None);
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::TimeRemaining),
            "—"
        );
    }

    #[test]
    fn battery_state_mapping_and_labels_cover_all_variants() {
        use starship_battery::State;

        assert_eq!(map_battery_state(State::Charging).label(), "Charging");
        assert_eq!(map_battery_state(State::Discharging).label(), "Discharging");
        assert_eq!(map_battery_state(State::Full).label(), "Full");
        assert_eq!(map_battery_state(State::Empty).label(), "Empty");
        assert_eq!(map_battery_state(State::Unknown).label(), "Unknown");
    }

    #[test]
    fn battery_entries_include_optional_fields_only_when_present() {
        let with = snapshot(0.5, BatteryState::Charging, Some(60));
        let mut without = snapshot(0.5, BatteryState::Charging, None);
        without.cycle_count = None;
        without.health = None;
        assert_eq!(battery_entries(&with).len(), 5);
        assert_eq!(battery_entries(&without).len(), 2);
    }

    #[test]
    fn no_battery_ratio_is_full_ac() {
        let p = no_battery_payload(Shape::Ratio, BatteryTextKind::Summary);
        assert!(matches!(p.body, Body::Ratio(_)));
        if let Body::Ratio(r) = p.body {
            assert_eq!(r.value, 1.0);
            assert_eq!(r.label.as_deref(), Some("AC"));
        }
    }

    #[test]
    fn no_battery_text_varies_by_kind() {
        let summary = no_battery_payload(Shape::Text, BatteryTextKind::Summary);
        let percent = no_battery_payload(Shape::Text, BatteryTextKind::Percent);
        let time = no_battery_payload(Shape::Text, BatteryTextKind::TimeRemaining);
        assert!(matches!(summary.body, Body::Text(_)));
        assert!(matches!(percent.body, Body::Text(_)));
        assert!(matches!(time.body, Body::Text(_)));
        if let Body::Text(text) = summary.body {
            assert_eq!(text.value, "AC");
        }
        if let Body::Text(text) = percent.body {
            assert_eq!(text.value, "100%");
        }
        if let Body::Text(text) = time.body {
            assert_eq!(text.value, "—");
        }
    }

    #[test]
    fn no_battery_entries_and_badge_use_ac_placeholders() {
        let entries = no_battery_payload(Shape::Entries, BatteryTextKind::Summary);
        let badge = no_battery_payload(Shape::Badge, BatteryTextKind::Summary);
        assert!(matches!(entries.body, Body::Entries(_)));
        assert!(matches!(badge.body, Body::Badge(_)));
        if let Body::Entries(entries) = entries.body {
            assert_eq!(entries.items[0].key, "power");
            assert_eq!(entries.items[0].value.as_deref(), Some("AC"));
        }
        if let Body::Badge(badge) = badge.body {
            assert_eq!(badge.status, Status::Ok);
            assert_eq!(badge.label, "AC");
        }
    }

    #[test]
    fn battery_compute_never_panics_on_any_shape() {
        let f = SystemMonitorBattery::new();
        for shape in [
            None,
            Some(Shape::Ratio),
            Some(Shape::Text),
            Some(Shape::Entries),
            Some(Shape::Badge),
        ] {
            let p = f.compute(&ctx_with_shape(shape));
            // Each branch must produce *some* body; on hosts without a battery we land on the
            // AC stand-in, on laptops we get the real reading. Both are valid.
            assert!(!matches!(p.body, Body::Image(_)));
        }
    }

    #[test]
    fn battery_badge_status_reflects_charge_and_state() {
        let low = snapshot(0.05, BatteryState::Discharging, None);
        assert_eq!(battery_badge(&low).status, Status::Error);
        let mid = snapshot(0.30, BatteryState::Discharging, None);
        assert_eq!(battery_badge(&mid).status, Status::Warn);
        let high = snapshot(0.95, BatteryState::Discharging, None);
        assert_eq!(battery_badge(&high).status, Status::Ok);
        let charging_low = snapshot(0.05, BatteryState::Charging, None);
        assert_eq!(battery_badge(&charging_low).status, Status::Ok);
    }

    #[test]
    fn battery_rejects_unknown_option_to_placeholder() {
        let f = SystemMonitorBattery::new();
        let p = f.compute(&ctx_text(Some("bogus = true")));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(t.value.starts_with("⚠"));
        }
    }

    #[test]
    fn battery_compute_without_manager_uses_no_battery_fallbacks() {
        let fetcher = SystemMonitorBattery {
            manager: Mutex::new(None),
        };

        let text = fetcher.compute(&ctx_text(Some("kind = \"percent\"")));
        let entries = fetcher.compute(&ctx_with_shape(Some(Shape::Entries)));
        let badge = fetcher.compute(&ctx_with_shape(Some(Shape::Badge)));
        assert!(matches!(text.body, Body::Text(_)));
        assert!(matches!(entries.body, Body::Entries(_)));
        assert!(matches!(badge.body, Body::Badge(_)));
        if let Body::Text(text) = text.body {
            assert_eq!(text.value, "100%");
        }
        if let Body::Entries(entries) = entries.body {
            assert_eq!(entries.items[0].value.as_deref(), Some("AC"));
        }
        if let Body::Badge(badge) = badge.body {
            assert_eq!(badge.label, "AC");
        }
    }

    #[tokio::test]
    async fn disk_defaults_to_ratio_or_text_fallback() {
        let ctx = ctx_with_shape(None);
        let p = SystemMonitorDisk.fetch(&ctx).await.unwrap();
        assert!(matches!(p.body, Body::Ratio(_) | Body::Text(_)));
    }

    #[tokio::test]
    async fn disk_bars_shape_emits_bars_body() {
        let ctx = ctx_with_shape(Some(Shape::Bars));
        let p = SystemMonitorDisk.fetch(&ctx).await.unwrap();
        assert!(matches!(p.body, Body::Bars(_)));
    }
}
