//! `system_info_kernel` — kernel name and version.

use serde::Deserialize;
use sysinfo::System;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{kernel_name, options_placeholder, parse_options, payload};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"name\" | \"version\"",
    required: false,
    default: Some("\"name\""),
    description: "Selects kernel name (`Linux` / `Darwin` / `Windows NT` / ...) or version string.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KernelInfoOptions {
    #[serde(default)]
    pub kind: Option<KernelInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelInfoKind {
    #[default]
    Name,
    Version,
}

pub struct SystemInfoKernel;

impl RealtimeFetcher for SystemInfoKernel {
    fn name(&self) -> &str {
        "system_info_kernel"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Kernel name (`Linux` / `Darwin` / `Windows NT` / ...) or version string from sysinfo."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("Linux")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: KernelInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            KernelInfoKind::Name => kernel_name(),
            KernelInfoKind::Version => System::kernel_version().unwrap_or_else(|| "unknown".into()),
        };
        payload(Body::Text(TextData { value }))
    }
}
