//! `system_monitor_host` — host snapshot rollup (os / host / uptime / load / cpu% / mem%).
//!
//! Entries by default; TextBlock collapses each row to "key: value". For a single static
//! identifier (terminal / shell / arch / ...) use `system_info_host`.

use std::sync::Mutex;

use sysinfo::System;

use crate::payload::{Body, EntriesData, Payload, TextBlockData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, format_uptime, memory_ratio, os_label, payload};

pub struct SystemMonitorHost {
    state: Mutex<System>,
    os: String,
    host: String,
}

impl SystemMonitorHost {
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

impl Default for SystemMonitorHost {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemMonitorHost {
    fn name(&self) -> &str {
        "system_monitor_host"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Host snapshot rollup combining OS / hostname / uptime / load / CPU% / memory% into one block. `Entries` is the default; `TextBlock` collapses each row to `\"key: value\"`. For a single static identifier (terminal / shell / arch / etc) use `system_info_host`."
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
        let mut sys = self.state.lock().unwrap_or_else(|e| e.into_inner());
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

#[cfg(test)]
mod tests {
    use super::super::test_helpers::ctx_with_shape;
    use super::*;

    #[test]
    fn rollup_emits_six_rows() {
        let p = SystemMonitorHost::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Entries(_)));
        if let Body::Entries(e) = p.body {
            assert_eq!(e.items.len(), 6);
        }
    }

    #[test]
    fn text_block_shape_returns_key_value_strings() {
        let p = SystemMonitorHost::new().compute(&ctx_with_shape(Some(Shape::TextBlock)));
        assert!(matches!(p.body, Body::TextBlock(_)));
        if let Body::TextBlock(l) = p.body {
            assert_eq!(l.lines.len(), 6);
            assert!(l.lines.iter().all(|s| s.contains(": ")));
        }
    }
}
