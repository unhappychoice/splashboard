//! `system_monitor_uptime` — time since the host last booted.

use sysinfo::System;

use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{format_uptime, payload};

pub struct SystemMonitorUptime;

impl RealtimeFetcher for SystemMonitorUptime {
    fn name(&self) -> &str {
        "system_monitor_uptime"
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
    fn compute(&self, _ctx: &FetchContext) -> Payload {
        payload(Body::Text(TextData {
            value: format_uptime(System::uptime()),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::ctx_with_shape;
    use super::*;

    #[test]
    fn uptime_emits_text() {
        let p = SystemMonitorUptime.compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(!t.value.is_empty());
        }
    }
}
