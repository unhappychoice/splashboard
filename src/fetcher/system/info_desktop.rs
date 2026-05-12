//! `system_info_desktop` — graphical session identity (DE / WM / init).

use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{
    detect_desktop_environment, detect_init_system, detect_window_manager, options_placeholder,
    parse_options, payload,
};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"de\" | \"wm\" | \"init\"",
    required: false,
    default: Some("\"de\""),
    description: "Selects desktop session identifier: desktop environment, window manager protocol, or init system.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopInfoOptions {
    #[serde(default)]
    pub kind: Option<DesktopInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopInfoKind {
    #[default]
    De,
    Wm,
    Init,
}

pub struct SystemInfoDesktop;

impl RealtimeFetcher for SystemInfoDesktop {
    fn name(&self) -> &str {
        "system_info_desktop"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Desktop session identifier: desktop environment (`$XDG_CURRENT_DESKTOP` family), window manager protocol (wayland / x11 / Quartz / DWM), or init system (`systemd` / `launchd` / `wininit`)."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("GNOME")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: DesktopInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            DesktopInfoKind::De => detect_desktop_environment(),
            DesktopInfoKind::Wm => detect_window_manager(),
            DesktopInfoKind::Init => detect_init_system(),
        };
        payload(Body::Text(TextData { value }))
    }
}
