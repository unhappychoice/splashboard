//! `system_info_cpu` — static CPU identifier (model / cores / frequency / vendor).
//!
//! Cached after the first read since these don't change after boot. For dynamic usage use
//! `system_monitor_cpu`.

use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{
    cached_cpu_info, format_cpu_cores, format_cpu_frequency, options_placeholder, parse_options,
    payload,
};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"model\" | \"cores\" | \"frequency\" | \"vendor\"",
    required: false,
    default: Some("\"model\""),
    description: "Selects which static CPU identifier the `Text` shape emits.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuInfoOptions {
    #[serde(default)]
    pub kind: Option<CpuInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuInfoKind {
    #[default]
    Model,
    Cores,
    Frequency,
    Vendor,
}

pub struct SystemInfoCpu;

impl RealtimeFetcher for SystemInfoCpu {
    fn name(&self) -> &str {
        "system_info_cpu"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Static CPU identifier: model name / core count / base frequency / vendor. Use for hero or attribution lines; pair with `system_monitor_cpu` for live load."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("Apple M3 Pro")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: CpuInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let info = cached_cpu_info();
        let value = match opts.kind.unwrap_or_default() {
            CpuInfoKind::Model => info.model.clone(),
            CpuInfoKind::Cores => format_cpu_cores(),
            CpuInfoKind::Frequency => format_cpu_frequency(info.frequency_mhz),
            CpuInfoKind::Vendor => info.vendor.clone(),
        };
        payload(Body::Text(TextData { value }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::ctx_text;
    use super::*;

    #[test]
    fn defaults_to_model_kind() {
        let p = SystemInfoCpu.compute(&ctx_text(None));
        assert!(matches!(p.body, Body::Text(t) if !t.value.is_empty()));
    }

    #[test]
    fn cores_kind_returns_non_empty_text() {
        let p = SystemInfoCpu.compute(&ctx_text(Some("kind = \"cores\"")));
        assert!(matches!(p.body, Body::Text(t) if !t.value.is_empty()));
    }

    #[test]
    fn frequency_kind_returns_non_empty_text() {
        let p = SystemInfoCpu.compute(&ctx_text(Some("kind = \"frequency\"")));
        assert!(matches!(p.body, Body::Text(t) if !t.value.is_empty()));
    }

    #[test]
    fn vendor_kind_returns_non_empty_text() {
        let p = SystemInfoCpu.compute(&ctx_text(Some("kind = \"vendor\"")));
        assert!(matches!(p.body, Body::Text(t) if !t.value.is_empty()));
    }

    #[test]
    fn rejects_unknown_kind_to_placeholder() {
        let p = SystemInfoCpu.compute(&ctx_text(Some("kind = \"bogus\"")));
        assert!(matches!(p.body, Body::Text(t) if t.value.starts_with('⚠')));
    }
}
