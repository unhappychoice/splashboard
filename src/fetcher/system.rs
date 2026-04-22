//! Cross-platform system fetchers backed by `sysinfo`.
//!
//! All are `Safety::Safe` — local kernel counters only, no network or exec. Realtime fetchers
//! cache a `Mutex<System>` and refresh only the fields they need per frame, so the `<1ms
//! infallible` contract holds even as many widgets sample the same source.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sysinfo::{Disks, ProcessesToUpdate, System};

use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, Payload, RatioData, TextBlockData, TextData,
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
    ]
}

pub fn cached_fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(DiskFetcher)]
}

/// `os / host / uptime / load / cpu / memory` rollup. `Entries` by default; `TextBlock`
/// collapses each row to `"key: value"` so the same fetcher can feed the plain text renderer.
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
    fn shapes(&self) -> &[Shape] {
        &[Shape::Entries, Shape::TextBlock]
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
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
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
        "disk_usage"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
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
    use std::time::Duration;

    fn ctx_with_shape(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            timeout: Duration::from_secs(1),
            shape,
            ..Default::default()
        }
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
