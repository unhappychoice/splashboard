//! Cross-platform system fetchers backed by `sysinfo`.
//!
//! All are `Safety::Safe` — local kernel counters only, no network or exec. Realtime fetchers
//! cache a `Mutex<System>` and refresh only the fields they need per frame, so the `<1ms
//! infallible` contract holds even as many widgets sample the same source.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::Deserialize;
use sysinfo::{Disks, ProcessesToUpdate, System};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, Payload, RatioData, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;
use crate::samples;

use super::{FetchContext, FetchError, Fetcher, RealtimeFetcher, Safety};

pub fn realtime_fetchers() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![
        Arc::new(SystemFetcher::new()),
        Arc::new(CpuLoadFetcher::new()),
        Arc::new(MemoryFetcher::new()),
        Arc::new(UptimeFetcher),
        Arc::new(LoadAverageFetcher),
        Arc::new(ProcessTopFetcher::new()),
        Arc::new(BatteryFetcher::new()),
    ]
}

pub fn cached_fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(DiskFetcher)]
}

const SYSTEM_OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"terminal\" | \"os\" | \"os_version\" | \"hostname\" | \"shell\" | \"arch\"",
    required: false,
    default: Some("\"terminal\""),
    description: "Selects the single value emitted by the `Text` shape. Ignored by `Entries` / `TextBlock` shapes, which always return the full rollup.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemOptions {
    #[serde(default)]
    pub kind: Option<SystemKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemKind {
    #[default]
    Terminal,
    Os,
    OsVersion,
    Hostname,
    Shell,
    Arch,
}

/// `os / host / uptime / load / cpu / memory` rollup. `Entries` by default; `TextBlock`
/// collapses each row to `"key: value"`; `Text` emits a single identity field selected by the
/// `kind` option (terminal name, OS label, hostname, shell, arch) for hero / attribution lines.
pub struct SystemFetcher {
    state: Mutex<System>,
    os: String,
    host: String,
}

impl SystemFetcher {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        Self {
            state: Mutex::new(sys),
            os: os_label(),
            host: System::host_name().unwrap_or_else(|| "unknown".into()),
        }
    }
}

impl Default for SystemFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemFetcher {
    fn name(&self) -> &str {
        "system"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Host identity rollup combining OS, hostname, uptime, load, CPU, and memory into one block. Use the `kind` option on the `Text` shape to extract a single field (terminal, OS, hostname, shell, arch) for hero or attribution lines."
    }
    fn shapes(&self) -> &[Shape] {
        // Listed specific-to-broad so multi-shape renderers (text_plain, animated_postfx)
        // pick `Text` by default — that's the variant where `kind = "terminal" | "os" | …`
        // takes effect, which is what hero / attribution widgets almost always want. The
        // full rollup still lands via `render = "grid_table"` (Entries-only renderer).
        &[Shape::Text, Shape::TextBlock, Shape::Entries]
    }
    fn default_shape(&self) -> Shape {
        // Preserve "no render spec = full rollup". Widgets that omit `render = ...` pick up
        // the Entries view (CPU / memory / uptime / …); the reordered `shapes()` only
        // affects intersection with multi-shape renderers.
        Shape::Entries
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        SYSTEM_OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("os", "linux"),
                ("host", "dev"),
                ("uptime", "3d 4h"),
                ("load", "0.42"),
                ("cpu", "18%"),
                ("memory", "67%"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "os: linux",
                "host: dev",
                "uptime: 3d 4h",
                "load: 0.42",
                "cpu: 18%",
                "memory: 67%",
            ]),
            Shape::Text => samples::text("iTerm2"),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        if matches!(ctx.shape, Some(Shape::Text)) {
            return self.compute_text(ctx);
        }
        let mut sys = self.state.lock().expect("system state mutex poisoned");
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        let rows = [
            ("os", self.os.clone()),
            ("host", self.host.clone()),
            ("uptime", format_uptime(System::uptime())),
            ("load", format!("{:.2}", System::load_average().one)),
            ("cpu", format!("{:.0}%", sys.global_cpu_usage())),
            ("memory", format!("{:.0}%", memory_ratio(&sys) * 100.0)),
        ];
        match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::TextBlock => payload(Body::TextBlock(TextBlockData {
                lines: rows.iter().map(|(k, v)| format!("{k}: {v}")).collect(),
            })),
            _ => payload(Body::Entries(EntriesData {
                items: rows.iter().map(|(k, v)| entry(k, v)).collect(),
            })),
        }
    }
}

impl SystemFetcher {
    fn compute_text(&self, ctx: &FetchContext) -> Payload {
        let opts: SystemOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = resolve_system_kind(opts.kind.unwrap_or_default());
        payload(Body::Text(TextData { value }))
    }
}

fn resolve_system_kind(kind: SystemKind) -> String {
    match kind {
        SystemKind::Terminal => detect_terminal(),
        SystemKind::Os => System::name().unwrap_or_else(|| "unknown".into()),
        SystemKind::OsVersion => os_label(),
        SystemKind::Hostname => System::host_name().unwrap_or_else(|| "unknown".into()),
        SystemKind::Shell => detect_shell(),
        SystemKind::Arch => std::env::consts::ARCH.into(),
    }
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
        _ => "terminal".into(),
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
        .unwrap_or_else(|| "shell".into())
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

/// Aggregated CPU usage across all cores. `Ratio` (0..=1) for gauges, `Text` for plain text.
pub struct CpuLoadFetcher {
    state: Mutex<System>,
}

impl CpuLoadFetcher {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        Self {
            state: Mutex::new(sys),
        }
    }
}

impl Default for CpuLoadFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for CpuLoadFetcher {
    fn name(&self) -> &str {
        "system_cpu"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Aggregated CPU usage across all cores, sampled every frame. Pair with a gauge renderer for a live meter or use the `Text` shape for a plain percentage."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.42, "cpu"),
            Shape::Text => samples::text("42%"),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let mut sys = self.state.lock().expect("cpu state mutex poisoned");
        sys.refresh_cpu_usage();
        let pct = sys.global_cpu_usage();
        let ratio = (f64::from(pct) / 100.0).clamp(0.0, 1.0);
        let label = format!("{pct:.0}%");
        match ctx.shape.unwrap_or(Shape::Ratio) {
            Shape::Text => payload(Body::Text(TextData { value: label })),
            _ => payload(Body::Ratio(RatioData {
                value: ratio,
                label: Some(label),
                denominator: None,
            })),
        }
    }
}

/// RAM usage. `Ratio` by default, `Text` as `"3.2 GB / 16 GB"`, `Entries` as used/total/free rows.
pub struct MemoryFetcher {
    state: Mutex<System>,
}

impl MemoryFetcher {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_memory();
        Self {
            state: Mutex::new(sys),
        }
    }
}

impl Default for MemoryFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for MemoryFetcher {
    fn name(&self) -> &str {
        "system_memory"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "RAM utilisation as a used/total ratio. `Text` formats as `\"6.4 GiB / 16 GiB\"` and `Entries` breaks it into used / total / free rows."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text, Shape::Entries]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.67, "memory"),
            Shape::Text => samples::text("6.4 GiB / 16 GiB"),
            Shape::Entries => samples::entries(&[
                ("used", "6.4 GiB"),
                ("total", "16 GiB"),
                ("free", "9.6 GiB"),
            ]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let mut sys = self.state.lock().expect("memory state mutex poisoned");
        sys.refresh_memory();
        let total = sys.total_memory();
        let used = sys.used_memory();
        let ratio = ratio_of(used, total);
        let label = format!("{} / {}", format_bytes(used), format_bytes(total));
        match ctx.shape.unwrap_or(Shape::Ratio) {
            Shape::Text => payload(Body::Text(TextData { value: label })),
            Shape::Entries => payload(Body::Entries(EntriesData {
                items: vec![
                    entry("used", &format_bytes(used)),
                    entry("total", &format_bytes(total)),
                    entry("free", &format_bytes(total.saturating_sub(used))),
                ],
            })),
            _ => payload(Body::Ratio(RatioData {
                value: ratio,
                label: Some(label),
                denominator: None,
            })),
        }
    }
}

/// Time since boot as a compact `"3d 4h"` / `"2h 15m"` / `"45m"` string.
pub struct UptimeFetcher;

impl RealtimeFetcher for UptimeFetcher {
    fn name(&self) -> &str {
        "system_uptime"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Time since the host last booted, formatted as a compact `\"3d 4h\"` / `\"2h 15m\"` / `\"45m\"` string."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("3d 4h 12m")),
            _ => None,
        }
    }
    fn compute(&self, _: &FetchContext) -> Payload {
        payload(Body::Text(TextData {
            value: format_uptime(System::uptime()),
        }))
    }
}

/// 1 / 5 / 15-minute load average. `Text` default; `Entries` splits the three windows.
/// Windows doesn't expose load average — shown as `"n/a (windows)"` there.
pub struct LoadAverageFetcher;

impl RealtimeFetcher for LoadAverageFetcher {
    fn name(&self) -> &str {
        "system_load"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Unix 1 / 5 / 15-minute load averages. `Text` joins the three values on one line; `Entries` splits them into separate rows. Reads as `\"n/a (windows)\"` on Windows, which has no equivalent counter."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text, Shape::Entries]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("0.42  0.38  0.31"),
            Shape::Entries => {
                samples::entries(&[("1min", "0.42"), ("5min", "0.38"), ("15min", "0.31")])
            }
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let la = System::load_average();
        match ctx.shape.unwrap_or(Shape::Text) {
            Shape::Entries => payload(Body::Entries(EntriesData {
                items: vec![
                    entry("1min", &format_load(la.one)),
                    entry("5min", &format_load(la.five)),
                    entry("15min", &format_load(la.fifteen)),
                ],
            })),
            _ => payload(Body::Text(TextData {
                value: load_line(la.one, la.five, la.fifteen),
            })),
        }
    }
}

/// Top N processes by CPU usage. `Entries` default (`"python": "42.1%"`), `TextBlock` collapses
/// to one process per row.
pub struct ProcessTopFetcher {
    state: Mutex<System>,
}

const PROCESS_TOP_COUNT: usize = 5;

impl ProcessTopFetcher {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        Self {
            state: Mutex::new(sys),
        }
    }
}

impl Default for ProcessTopFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for ProcessTopFetcher {
    fn name(&self) -> &str {
        "system_processes"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top five processes by current CPU usage, refreshed every frame. `Entries` pairs each process name with its percentage; `TextBlock` collapses to one process per line."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Entries, Shape::TextBlock]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("node", "12.4%"),
                ("cargo", "8.1%"),
                ("firefox", "6.3%"),
                ("zsh", "2.1%"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "node       12.4%",
                "cargo       8.1%",
                "firefox     6.3%",
                "zsh         2.1%",
            ]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let mut sys = self.state.lock().expect("process state mutex poisoned");
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let rows = top_processes(&sys, PROCESS_TOP_COUNT);
        match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::TextBlock => payload(Body::TextBlock(TextBlockData {
                lines: rows.iter().map(|(n, c)| format!("{n}  {c:.1}%")).collect(),
            })),
            _ => payload(Body::Entries(EntriesData {
                items: rows
                    .iter()
                    .map(|(n, c)| entry(n, &format!("{c:.1}%")))
                    .collect(),
            })),
        }
    }
}

/// Disk usage. Cached (mount scan on each refresh is a syscall, not <1ms). Defaults to the
/// largest disk as `Ratio`; `Text` renders `"45% of 500 GB"`; `Bars` lists every mount.
pub struct DiskFetcher;

#[async_trait]
impl Fetcher for DiskFetcher {
    fn name(&self) -> &str {
        "system_disk_usage"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Local disk usage. `Ratio` and `Text` summarise the largest mounted disk (used vs total); `Bars` lists every detected mount with its used bytes."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text, Shape::Bars]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.58, "disk"),
            Shape::Text => samples::text("58% of 400 GB"),
            Shape::Bars => samples::bars(&[("/", 42), ("/home", 110), ("/data", 200)]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let disks = Disks::new_with_refreshed_list();
        Ok(match ctx.shape.unwrap_or(Shape::Ratio) {
            Shape::Bars => payload(Body::Bars(BarsData {
                bars: disk_bars(&disks),
            })),
            Shape::Text => payload(Body::Text(TextData {
                value: primary_disk(&disks)
                    .map(|(t, a)| disk_label(t, a))
                    .unwrap_or_else(|| "no disks detected".into()),
            })),
            _ => primary_disk(&disks)
                .map(|(total, available)| {
                    let used = total.saturating_sub(available);
                    payload(Body::Ratio(RatioData {
                        value: ratio_of(used, total),
                        label: Some(disk_label(total, available)),
                        denominator: None,
                    }))
                })
                .unwrap_or_else(|| {
                    payload(Body::Text(TextData {
                        value: "no disks detected".into(),
                    }))
                }),
        })
    }
}

const BATTERY_OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "kind",
        type_hint: "\"summary\" | \"percent\" | \"status\" | \"time_remaining\"",
        required: false,
        default: Some("\"summary\""),
        description: "Selects the format of the `Text` shape. Ignored by `Ratio` / `Entries`.",
    },
    OptionSchema {
        name: "index",
        type_hint: "integer",
        required: false,
        default: Some("0"),
        description: "Index of the battery to read on multi-battery systems.",
    },
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatteryOptions {
    #[serde(default)]
    pub kind: Option<BatteryTextKind>,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryTextKind {
    #[default]
    Summary,
    Percent,
    Status,
    TimeRemaining,
}

#[derive(Clone, Copy)]
enum BatteryState {
    Charging,
    Discharging,
    Full,
    Empty,
    Unknown,
}

impl BatteryState {
    fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Full => "Full",
            Self::Empty => "Empty",
            Self::Unknown => "Unknown",
        }
    }
}

struct BatterySnapshot {
    charge: f64,
    state: BatteryState,
    time_remaining_secs: Option<u64>,
    cycle_count: Option<u32>,
    health: Option<f64>,
}

/// Primary (or selected) battery state. `Ratio` pairs with `gauge_battery`; `Text` is a
/// formatted summary (`kind` picks the field); `Entries` rolls up charge / state / time / cycles
/// / health. Hosts without a battery (desktops, servers) render as a "full AC" stand-in so the
/// widget doesn't disappear.
pub struct BatteryFetcher {
    manager: Mutex<Option<starship_battery::Manager>>,
}

impl BatteryFetcher {
    pub fn new() -> Self {
        Self {
            manager: Mutex::new(starship_battery::Manager::new().ok()),
        }
    }
}

impl Default for BatteryFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for BatteryFetcher {
    fn name(&self) -> &str {
        "system_battery"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Charge level and state of the primary (or `index`-selected) battery. `Ratio` drives gauges, `Text` formats a summary line whose field is picked by `kind`, and `Entries` rolls up charge / state / time-left / cycles / health. Hosts without a battery render a steady `\"AC\"` placeholder."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text, Shape::Entries, Shape::Badge]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        BATTERY_OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.87, "battery"),
            Shape::Text => samples::text("87% • Charging • 1h 23m"),
            Shape::Entries => samples::entries(&[
                ("charge", "87%"),
                ("state", "Charging"),
                ("time_left", "1h 23m"),
                ("cycles", "284"),
                ("health", "97%"),
            ]),
            Shape::Badge => samples::badge(Status::Ok, "87% · Charging"),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: BatteryOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let snapshot = self.read_snapshot(opts.index.unwrap_or(0));
        let shape = ctx.shape.unwrap_or(Shape::Ratio);
        match (snapshot, shape) {
            (Some(snap), Shape::Text) => payload(Body::Text(TextData {
                value: format_battery_text(&snap, opts.kind.unwrap_or_default()),
            })),
            (Some(snap), Shape::Entries) => payload(Body::Entries(EntriesData {
                items: battery_entries(&snap),
            })),
            (Some(snap), Shape::Badge) => payload(Body::Badge(battery_badge(&snap))),
            (Some(snap), _) => payload(Body::Ratio(RatioData {
                value: snap.charge,
                label: Some(format!(
                    "{} • {}",
                    format_percent(snap.charge),
                    snap.state.label()
                )),
                denominator: None,
            })),
            (None, shape) => no_battery_payload(shape, opts.kind.unwrap_or_default()),
        }
    }
}

impl BatteryFetcher {
    fn read_snapshot(&self, index: usize) -> Option<BatterySnapshot> {
        let manager = self.manager.lock().expect("battery manager mutex poisoned");
        let manager = manager.as_ref()?;
        let battery = manager.batteries().ok()?.nth(index)?.ok()?;
        Some(snapshot_from(&battery))
    }
}

fn snapshot_from(battery: &starship_battery::Battery) -> BatterySnapshot {
    BatterySnapshot {
        charge: f64::from(battery.state_of_charge().value).clamp(0.0, 1.0),
        state: map_battery_state(battery.state()),
        time_remaining_secs: time_remaining_secs(battery),
        cycle_count: battery.cycle_count(),
        health: battery_health(battery),
    }
}

fn map_battery_state(s: starship_battery::State) -> BatteryState {
    use starship_battery::State as S;
    match s {
        S::Charging => BatteryState::Charging,
        S::Discharging => BatteryState::Discharging,
        S::Full => BatteryState::Full,
        S::Empty => BatteryState::Empty,
        _ => BatteryState::Unknown,
    }
}

fn time_remaining_secs(battery: &starship_battery::Battery) -> Option<u64> {
    let dur = match battery.state() {
        starship_battery::State::Charging => battery.time_to_full(),
        starship_battery::State::Discharging => battery.time_to_empty(),
        _ => None,
    }?;
    Some(dur.value.max(0.0) as u64)
}

fn battery_health(battery: &starship_battery::Battery) -> Option<f64> {
    let full = f64::from(battery.energy_full().value);
    let design = f64::from(battery.energy_full_design().value);
    if design <= 0.0 {
        None
    } else {
        Some((full / design).clamp(0.0, 1.0))
    }
}

fn format_battery_text(snap: &BatterySnapshot, kind: BatteryTextKind) -> String {
    match kind {
        BatteryTextKind::Percent => format_percent(snap.charge),
        BatteryTextKind::Status => snap.state.label().into(),
        BatteryTextKind::TimeRemaining => snap
            .time_remaining_secs
            .map(format_uptime)
            .unwrap_or_else(|| "—".into()),
        BatteryTextKind::Summary => match snap.time_remaining_secs {
            Some(secs) => format!(
                "{} • {} • {}",
                format_percent(snap.charge),
                snap.state.label(),
                format_uptime(secs)
            ),
            None => format!("{} • {}", format_percent(snap.charge), snap.state.label()),
        },
    }
}

fn battery_badge(snap: &BatterySnapshot) -> BadgeData {
    let status = match snap.state {
        BatteryState::Charging | BatteryState::Full => Status::Ok,
        _ if snap.charge < 0.20 => Status::Error,
        _ if snap.charge < 0.50 => Status::Warn,
        _ => Status::Ok,
    };
    BadgeData {
        status,
        label: format!("{} · {}", format_percent(snap.charge), snap.state.label()),
    }
}

fn battery_entries(snap: &BatterySnapshot) -> Vec<Entry> {
    let mut items = vec![
        entry("charge", &format_percent(snap.charge)),
        entry("state", snap.state.label()),
    ];
    if let Some(secs) = snap.time_remaining_secs {
        items.push(entry("time_left", &format_uptime(secs)));
    }
    if let Some(cycles) = snap.cycle_count {
        items.push(entry("cycles", &cycles.to_string()));
    }
    if let Some(h) = snap.health {
        items.push(entry("health", &format_percent(h)));
    }
    items
}

fn no_battery_payload(shape: Shape, kind: BatteryTextKind) -> Payload {
    match shape {
        Shape::Text => payload(Body::Text(TextData {
            value: match kind {
                BatteryTextKind::Percent => "100%".into(),
                BatteryTextKind::TimeRemaining => "—".into(),
                _ => "AC".into(),
            },
        })),
        Shape::Entries => payload(Body::Entries(EntriesData {
            items: vec![entry("power", "AC")],
        })),
        Shape::Badge => payload(Body::Badge(BadgeData {
            status: Status::Ok,
            label: "AC".into(),
        })),
        _ => payload(Body::Ratio(RatioData {
            value: 1.0,
            label: Some("AC".into()),
            denominator: None,
        })),
    }
}

fn format_percent(ratio: f64) -> String {
    format!("{:.0}%", ratio.clamp(0.0, 1.0) * 100.0)
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
    use super::*;
    use crate::paths::TEST_ENV_LOCK;
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
                "system",
                "system_cpu",
                "system_memory",
                "system_uptime",
                "system_load",
                "system_processes",
                "system_battery",
            ]
        );
        assert_eq!(cached_names, vec!["system_disk_usage"]);
    }

    #[test]
    fn fetcher_contracts_cover_supported_shapes_and_samples() {
        assert_realtime_contract(
            &SystemFetcher::default(),
            "system",
            &[Shape::Text, Shape::TextBlock, Shape::Entries],
            Shape::Entries,
            Shape::Ratio,
            1,
        );
        assert_realtime_contract(
            &CpuLoadFetcher::default(),
            "system_cpu",
            &[Shape::Ratio, Shape::Text],
            Shape::Ratio,
            Shape::Entries,
            0,
        );
        assert_realtime_contract(
            &MemoryFetcher::default(),
            "system_memory",
            &[Shape::Ratio, Shape::Text, Shape::Entries],
            Shape::Ratio,
            Shape::Bars,
            0,
        );
        assert_realtime_contract(
            &UptimeFetcher,
            "system_uptime",
            &[Shape::Text],
            Shape::Text,
            Shape::Entries,
            0,
        );
        assert_realtime_contract(
            &LoadAverageFetcher,
            "system_load",
            &[Shape::Text, Shape::Entries],
            Shape::Text,
            Shape::Badge,
            0,
        );
        assert_realtime_contract(
            &ProcessTopFetcher::default(),
            "system_processes",
            &[Shape::Entries, Shape::TextBlock],
            Shape::Entries,
            Shape::Ratio,
            0,
        );
        assert_realtime_contract(
            &BatteryFetcher::default(),
            "system_battery",
            &[Shape::Ratio, Shape::Text, Shape::Entries, Shape::Badge],
            Shape::Ratio,
            Shape::Bars,
            2,
        );
        assert_cached_contract(
            &DiskFetcher,
            "system_disk_usage",
            &[Shape::Ratio, Shape::Text, Shape::Bars],
            Shape::Entries,
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
        assert_eq!(detect_terminal_with(&[]), "terminal");
    }

    #[test]
    fn detect_shell_uses_basename_and_fallback() {
        let _guard = EnvGuard::set(&[("SHELL", Some("/usr/local/bin/fish"))]);
        assert_eq!(detect_shell(), "fish");
        drop(_guard);

        let _guard = EnvGuard::set(&[("SHELL", None)]);
        assert_eq!(detect_shell(), "shell");
    }

    #[test]
    fn resolve_system_kind_covers_each_identity_variant() {
        let pairs: Vec<_> = TERMINAL_ENV_KEYS
            .iter()
            .copied()
            .map(|key| (key, None))
            .chain([("SHELL", Some("/bin/zsh")), ("TERM_PROGRAM", Some("Hyper"))])
            .collect();
        let _guard = EnvGuard::set(&pairs);

        assert_eq!(resolve_system_kind(SystemKind::Terminal), "Hyper");
        assert!(!resolve_system_kind(SystemKind::Os).is_empty());
        assert!(!resolve_system_kind(SystemKind::OsVersion).is_empty());
        assert!(!resolve_system_kind(SystemKind::Hostname).is_empty());
        assert_eq!(resolve_system_kind(SystemKind::Shell), "zsh");
        assert_eq!(
            resolve_system_kind(SystemKind::Arch),
            std::env::consts::ARCH
        );
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
    fn cpu_load_defaults_to_ratio() {
        let p = CpuLoadFetcher::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Ratio(_)));
    }

    #[test]
    fn cpu_load_emits_text_when_requested() {
        let p = CpuLoadFetcher::new().compute(&ctx_with_shape(Some(Shape::Text)));
        assert!(matches!(p.body, Body::Text(_)));
    }

    #[test]
    fn memory_defaults_to_ratio() {
        let p = MemoryFetcher::new().compute(&ctx_with_shape(None));
        let Body::Ratio(r) = p.body else {
            panic!("expected ratio");
        };
        assert!((0.0..=1.0).contains(&r.value));
    }

    #[test]
    fn memory_entries_shape_has_three_rows() {
        let p = MemoryFetcher::new().compute(&ctx_with_shape(Some(Shape::Entries)));
        let Body::Entries(e) = p.body else {
            panic!("expected entries");
        };
        let keys: Vec<_> = e.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["used", "total", "free"]);
    }

    #[test]
    fn memory_text_shape_formats_used_over_total() {
        let p = MemoryFetcher::new().compute(&ctx_with_shape(Some(Shape::Text)));
        let Body::Text(text) = p.body else {
            panic!("expected text");
        };
        assert!(text.value.contains(" / "));
    }

    #[test]
    fn uptime_emits_text() {
        let p = UptimeFetcher.compute(&ctx_with_shape(None));
        let Body::Text(t) = p.body else {
            panic!("expected text");
        };
        assert!(!t.value.is_empty());
    }

    #[test]
    fn load_average_defaults_to_text() {
        let p = LoadAverageFetcher.compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Text(_)));
    }

    #[test]
    fn load_average_entries_shape_has_three_windows() {
        let p = LoadAverageFetcher.compute(&ctx_with_shape(Some(Shape::Entries)));
        let Body::Entries(e) = p.body else {
            panic!("expected entries");
        };
        let keys: Vec<_> = e.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["1min", "5min", "15min"]);
    }

    #[test]
    fn system_rollup_emits_six_rows() {
        let p = SystemFetcher::new().compute(&ctx_with_shape(None));
        let Body::Entries(e) = p.body else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 6);
    }

    #[test]
    fn system_text_shape_defaults_to_terminal_kind() {
        let p = SystemFetcher::new().compute(&ctx_text(None));
        let Body::Text(t) = p.body else {
            panic!("expected text");
        };
        assert!(!t.value.is_empty());
    }

    #[test]
    fn system_text_shape_emits_arch_when_requested() {
        let p = SystemFetcher::new().compute(&ctx_text(Some("kind = \"arch\"")));
        let Body::Text(t) = p.body else {
            panic!("expected text");
        };
        assert_eq!(t.value, std::env::consts::ARCH);
    }

    #[test]
    fn system_text_shape_rejects_unknown_kind_to_placeholder() {
        let p = SystemFetcher::new().compute(&ctx_text(Some("kind = \"bogus\"")));
        let Body::Text(t) = p.body else {
            panic!("expected text");
        };
        assert!(t.value.starts_with("⚠"));
    }

    #[test]
    fn detect_terminal_returns_non_empty_label() {
        assert!(!detect_terminal().is_empty());
    }

    /// Prints one Text-shape line per `kind` on the host running the tests. Kept `#[ignore]` so
    /// the regular run stays side-effect free, but a dev can verify real output with
    /// `cargo test -- --ignored fetcher::system::tests::live_system_text_all_kinds --nocapture`.
    #[test]
    #[ignore]
    fn live_system_text_all_kinds() {
        let cases = [
            ("terminal", "kind = \"terminal\""),
            ("os", "kind = \"os\""),
            ("os_version", "kind = \"os_version\""),
            ("hostname", "kind = \"hostname\""),
            ("shell", "kind = \"shell\""),
            ("arch", "kind = \"arch\""),
        ];
        for (label, opts) in cases {
            let p = SystemFetcher::new().compute(&ctx_text(Some(opts)));
            let Body::Text(t) = p.body else {
                panic!("expected text for {label}");
            };
            eprintln!("{label:<12} → {}", t.value);
            assert!(!t.value.is_empty());
        }
    }

    #[test]
    fn system_text_block_shape_returns_key_value_strings() {
        let p = SystemFetcher::new().compute(&ctx_with_shape(Some(Shape::TextBlock)));
        let Body::TextBlock(l) = p.body else {
            panic!("expected text_block");
        };
        assert_eq!(l.lines.len(), 6);
        assert!(l.lines.iter().all(|s| s.contains(": ")));
    }

    #[test]
    fn process_top_respects_count_cap() {
        let p = ProcessTopFetcher::new().compute(&ctx_with_shape(None));
        let Body::Entries(e) = p.body else {
            panic!("expected entries");
        };
        assert!(e.items.len() <= PROCESS_TOP_COUNT);
    }

    #[test]
    fn process_top_text_block_shape_formats_rows() {
        let p = ProcessTopFetcher::new().compute(&ctx_with_shape(Some(Shape::TextBlock)));
        let Body::TextBlock(block) = p.body else {
            panic!("expected text block");
        };
        assert!(block.lines.len() <= PROCESS_TOP_COUNT);
        assert!(block.lines.iter().all(|line| line.ends_with('%')));
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
        let Body::Ratio(r) = p.body else {
            panic!("expected ratio")
        };
        assert_eq!(r.value, 1.0);
        assert_eq!(r.label.as_deref(), Some("AC"));
    }

    #[test]
    fn no_battery_text_varies_by_kind() {
        let summary = no_battery_payload(Shape::Text, BatteryTextKind::Summary);
        let percent = no_battery_payload(Shape::Text, BatteryTextKind::Percent);
        let time = no_battery_payload(Shape::Text, BatteryTextKind::TimeRemaining);
        let extract = |p: Payload| match p.body {
            Body::Text(t) => t.value,
            _ => panic!(),
        };
        assert_eq!(extract(summary), "AC");
        assert_eq!(extract(percent), "100%");
        assert_eq!(extract(time), "—");
    }

    #[test]
    fn no_battery_entries_and_badge_use_ac_placeholders() {
        let Body::Entries(entries) =
            no_battery_payload(Shape::Entries, BatteryTextKind::Summary).body
        else {
            panic!("expected entries");
        };
        let Body::Badge(badge) = no_battery_payload(Shape::Badge, BatteryTextKind::Summary).body
        else {
            panic!("expected badge");
        };
        assert_eq!(entries.items[0].key, "power");
        assert_eq!(entries.items[0].value.as_deref(), Some("AC"));
        assert_eq!(badge.status, Status::Ok);
        assert_eq!(badge.label, "AC");
    }

    #[test]
    fn battery_compute_never_panics_on_any_shape() {
        let f = BatteryFetcher::new();
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
        let f = BatteryFetcher::new();
        let p = f.compute(&ctx_text(Some("bogus = true")));
        let Body::Text(t) = p.body else {
            panic!("expected text placeholder")
        };
        assert!(t.value.starts_with("⚠"));
    }

    #[test]
    fn battery_compute_without_manager_uses_no_battery_fallbacks() {
        let fetcher = BatteryFetcher {
            manager: Mutex::new(None),
        };

        let Body::Text(text) = fetcher.compute(&ctx_text(Some("kind = \"percent\""))).body else {
            panic!("expected text");
        };
        let Body::Entries(entries) = fetcher.compute(&ctx_with_shape(Some(Shape::Entries))).body
        else {
            panic!("expected entries");
        };
        let Body::Badge(badge) = fetcher.compute(&ctx_with_shape(Some(Shape::Badge))).body else {
            panic!("expected badge");
        };

        assert_eq!(text.value, "100%");
        assert_eq!(entries.items[0].value.as_deref(), Some("AC"));
        assert_eq!(badge.label, "AC");
    }

    #[tokio::test]
    async fn disk_defaults_to_ratio_or_text_fallback() {
        let ctx = ctx_with_shape(None);
        let p = DiskFetcher.fetch(&ctx).await.unwrap();
        assert!(matches!(p.body, Body::Ratio(_) | Body::Text(_)));
    }

    #[tokio::test]
    async fn disk_bars_shape_emits_bars_body() {
        let ctx = ctx_with_shape(Some(Shape::Bars));
        let p = DiskFetcher.fetch(&ctx).await.unwrap();
        assert!(matches!(p.body, Body::Bars(_)));
    }
}
