//! `system_info_machine` — physical machine DMI identity.

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
    type_hint: "\"model\" | \"vendor\" | \"serial\" | \"chassis\"",
    required: false,
    default: Some("\"model\""),
    description: "Selects which physical-machine identifier the `Text` shape emits. Reads `/sys/class/dmi/id/*` on Linux; returns `\"n/a\"` on macOS / Windows.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineInfoOptions {
    #[serde(default)]
    pub kind: Option<MachineInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineInfoKind {
    #[default]
    Model,
    Vendor,
    Serial,
    Chassis,
}

pub struct SystemInfoMachine;

impl RealtimeFetcher for SystemInfoMachine {
    fn name(&self) -> &str {
        "system_info_machine"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Physical machine identity from DMI / SMBIOS: vendor / model / serial / chassis type. Linux reads `/sys/class/dmi/id/*`; non-Linux platforms render `\"n/a\"` until IOKit / WMI bindings land."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("MacBookPro18,3")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: MachineInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            MachineInfoKind::Model => dmi_or_na(dmi::host_model()),
            MachineInfoKind::Vendor => dmi_or_na(dmi::host_vendor()),
            MachineInfoKind::Serial => dmi_or_na(dmi::host_serial()),
            MachineInfoKind::Chassis => dmi_or_na(dmi::chassis()),
        };
        payload(Body::Text(TextData { value }))
    }
}
