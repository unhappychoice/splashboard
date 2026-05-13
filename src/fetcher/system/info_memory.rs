//! `system_info_memory` — installed memory totals (RAM / swap), byte-formatted.

use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{cached_memory_totals, format_bytes, options_placeholder, parse_options, payload};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"total\" | \"swap_total\"",
    required: false,
    default: Some("\"total\""),
    description: "Selects whether the `Text` shape emits installed RAM or swap-area total.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryInfoOptions {
    #[serde(default)]
    pub kind: Option<MemoryInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryInfoKind {
    #[default]
    Total,
    SwapTotal,
}

pub struct SystemInfoMemory;

impl RealtimeFetcher for SystemInfoMemory {
    fn name(&self) -> &str {
        "system_info_memory"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Installed memory capacity: RAM total or swap-area total, byte-formatted (`\"31.3 GB\"`)."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("16 GB")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: MemoryInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let totals = cached_memory_totals();
        let value = match opts.kind.unwrap_or_default() {
            MemoryInfoKind::Total => format_bytes(totals.memory),
            MemoryInfoKind::SwapTotal => format_bytes(totals.swap),
        };
        payload(Body::Text(TextData { value }))
    }
}
