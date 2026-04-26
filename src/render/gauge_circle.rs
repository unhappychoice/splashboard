use ratatui::{Frame, layout::Rect, style::Style, text::Span, widgets::Gauge};

use crate::options::OptionSchema;
use crate::payload::{Body, RatioData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

/// Reserved forward-compat fields. ratatui's current Gauge widget is a single full-height
/// bar so neither field has a visible effect yet; declared here for `deny_unknown_fields`
/// parity with the documented schema.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Options {
    #[serde(default)]
    pub ring_thickness: Option<u16>,
    #[serde(default)]
    pub label_position: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "ring_thickness",
        type_hint: "cells (u16)",
        required: false,
        default: None,
        description: "Reserved for the future ring variant — ratatui's current Gauge widget is a full-height bar, so this option is accepted but has no visible effect yet.",
    },
    OptionSchema {
        name: "label_position",
        type_hint: "\"center\" | \"below\"",
        required: false,
        default: Some("\"center\""),
        description: "Placement of the numeric label. `center` renders the label inside the bar; `below` is a no-op until the gauge gains a label slot beneath the fill.",
    },
];

pub struct GaugeCircleRenderer;

impl Renderer for GaugeCircleRenderer {
    fn name(&self) -> &str {
        "gauge_circle"
    }
    fn description(&self) -> &'static str {
        "Full-height block bar for `Ratio` built on ratatui's `Gauge`, with the optional label centred inside the fill. The chunkiest member of the `gauge_*` family — pick `gauge_line` when you need something single-row."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Ratio]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::Ratio(d) = body {
            // Parse extras so unknown keys still fail per `deny_unknown_fields`; values are
            // ignored until the underlying gauge widget supports them.
            let _: Options = opts.parse_specific();
            render_gauge(frame, area, d, theme);
        }
    }
}

fn render_gauge(frame: &mut Frame, area: Rect, data: &RatioData, theme: &Theme) {
    let ratio = data.value.clamp(0.0, 1.0);
    let mut gauge = Gauge::default().ratio(ratio);
    if let Some(label) = &data.label {
        gauge = gauge.label(Span::styled(label.clone(), Style::default().fg(theme.text)));
    }
    frame.render_widget(gauge, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, RatioData};
    use crate::render::test_utils::render_to_buffer;

    fn payload(value: f64, label: Option<&str>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Ratio(RatioData {
                value,
                label: label.map(String::from),
                denominator: None,
            }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let _ = render_to_buffer(&payload(0.5, Some("CPU")), 20, 5);
    }

    #[test]
    fn clamps_out_of_range_value() {
        let _ = render_to_buffer(&payload(1.7, None), 20, 5);
        let _ = render_to_buffer(&payload(-0.2, None), 20, 5);
    }
}
