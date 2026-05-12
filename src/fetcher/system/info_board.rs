//! `system_info_board` — motherboard DMI identity.

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
    type_hint: "\"vendor\" | \"model\"",
    required: false,
    default: Some("\"model\""),
    description: "Selects motherboard identifier. DMI-backed; `\"n/a\"` off Linux.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardInfoOptions {
    #[serde(default)]
    pub kind: Option<BoardInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardInfoKind {
    Vendor,
    #[default]
    Model,
}

pub struct SystemInfoBoard;

impl RealtimeFetcher for SystemInfoBoard {
    fn name(&self) -> &str {
        "system_info_board"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Motherboard vendor or model, via DMI. Linux only; other platforms render `\"n/a\"`."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("PRIME B660-PLUS")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: BoardInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            BoardInfoKind::Vendor => dmi_or_na(dmi::board_vendor()),
            BoardInfoKind::Model => dmi_or_na(dmi::board_model()),
        };
        payload(Body::Text(TextData { value }))
    }
}
