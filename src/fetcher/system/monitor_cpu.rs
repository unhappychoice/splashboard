//! `system_monitor_cpu` — aggregated CPU usage across all cores.

use std::sync::Mutex;

use sysinfo::System;

use crate::payload::{Body, Payload, RatioData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::payload;
use super::sprites;

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
        &[Shape::Ratio, Shape::Text, Shape::PixelArt]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.42, "cpu"),
            Shape::Text => samples::text("42%"),
            Shape::PixelArt => Body::PixelArt(sprites::cpu_sprite(0.42, "42%")),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let mut sys = self.state.lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_cpu_usage();
        let pct = sys.global_cpu_usage();
        let ratio = (f64::from(pct) / 100.0).clamp(0.0, 1.0);
        let label = format!("{pct:.0}%");
        match ctx.shape.unwrap_or(Shape::Ratio) {
            Shape::Text => payload(Body::Text(TextData { value: label })),
            Shape::PixelArt => payload(Body::PixelArt(sprites::cpu_sprite(ratio, &label))),
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

    #[test]
    fn cpu_load_emits_pixel_art_when_requested() {
        let p = SystemMonitorCpu::new().compute(&ctx_with_shape(Some(Shape::PixelArt)));
        let Body::PixelArt(d) = p.body else {
            panic!("expected PixelArt body");
        };
        assert_eq!(d.pixels.len(), 16);
        let label = d.label.unwrap();
        assert!(label.ends_with('%'), "label = {label}");
    }

    #[test]
    fn sample_body_covers_every_supported_shape() {
        let fetcher = SystemMonitorCpu::new();
        assert!(matches!(
            fetcher.sample_body(Shape::Ratio),
            Some(Body::Ratio(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Text),
            Some(Body::Text(_))
        ));
        let pixel = fetcher.sample_body(Shape::PixelArt).unwrap();
        let Body::PixelArt(d) = pixel else {
            panic!("expected PixelArt sample body");
        };
        assert_eq!(d.label.as_deref(), Some("42%"));
        assert!(fetcher.sample_body(Shape::Entries).is_none());
    }
}
