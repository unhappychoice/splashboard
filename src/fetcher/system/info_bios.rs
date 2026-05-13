//! `system_info_bios` — BIOS / UEFI firmware DMI identity.

use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::dmi;
use super::{dmi_or_na, options_placeholder, parse_options, payload};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"vendor\" | \"version\" | \"date\"",
    required: false,
    default: Some("\"version\""),
    description: "Selects BIOS/UEFI field. DMI-backed; `\"n/a\"` off Linux.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiosInfoOptions {
    #[serde(default)]
    pub kind: Option<BiosInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiosInfoKind {
    Vendor,
    #[default]
    Version,
    Date,
}

pub struct SystemInfoBios;

impl RealtimeFetcher for SystemInfoBios {
    fn name(&self) -> &str {
        "system_info_bios"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "BIOS / UEFI firmware identification (vendor / version / release date), via DMI. Linux only; other platforms render `\"n/a\"`."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("1.42.0")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: BiosInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            BiosInfoKind::Vendor => dmi_or_na(dmi::bios_vendor()),
            BiosInfoKind::Version => dmi_or_na(dmi::bios_version()),
            BiosInfoKind::Date => dmi_or_na(dmi::bios_date()),
        };
        payload(Body::Text(TextData { value }))
    }
}
