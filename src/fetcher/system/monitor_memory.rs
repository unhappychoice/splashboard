//! `system_monitor_memory` — RAM utilisation (Ratio / Text / Entries).

use std::sync::Mutex;

use sysinfo::System;

use crate::payload::{Body, EntriesData, Payload, RatioData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, format_bytes, payload, ratio_of};

pub struct SystemMonitorMemory {
    state: Mutex<System>,
}

impl SystemMonitorMemory {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_memory();
        Self {
            state: Mutex::new(sys),
        }
    }
}

impl Default for SystemMonitorMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemMonitorMemory {
    fn name(&self) -> &str {
        "system_monitor_memory"
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
