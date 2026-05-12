//! `system_info_env` — command preferences from environment ($EDITOR / $VISUAL / $PAGER).

use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{env_basename, options_placeholder, parse_options, payload};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"editor\" | \"visual\" | \"pager\"",
    required: false,
    default: Some("\"editor\""),
    description: "Selects which env-driven command preference the `Text` shape emits. Returns `\"(unset)\"` if the env var is empty.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvInfoOptions {
    #[serde(default)]
    pub kind: Option<EnvInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvInfoKind {
    #[default]
    Editor,
    Visual,
    Pager,
}

pub struct SystemInfoEnv;

impl RealtimeFetcher for SystemInfoEnv {
    fn name(&self) -> &str {
        "system_info_env"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Command preference from environment: `$EDITOR` / `$VISUAL` / `$PAGER`. Emits the binary basename (e.g. `nvim`), or `\"(unset)\"` when empty."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("nvim")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: EnvInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            EnvInfoKind::Editor => env_basename("EDITOR", "(unset)"),
            EnvInfoKind::Visual => env_basename("VISUAL", "(unset)"),
            EnvInfoKind::Pager => env_basename("PAGER", "(unset)"),
        };
        payload(Body::Text(TextData { value }))
    }
}
