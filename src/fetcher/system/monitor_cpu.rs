//! `system_monitor_cpu` — aggregated CPU usage across all cores.

use std::sync::Mutex;

use sysinfo::System;

use crate::payload::{Body, Payload, RatioData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::payload;

pub struct SystemMonitorCpu {
    state: Mutex<System>,
}

impl SystemMonitorCpu {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        Self {
            state: Mutex::new(sys),
        }
    }
}

impl Default for SystemMonitorCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemMonitorCpu {
    fn name(&self) -> &str {
        "system_monitor_cpu"
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

#[cfg(test)]
mod tests {
    use super::super::test_helpers::ctx_with_shape;
    use super::*;

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
}
