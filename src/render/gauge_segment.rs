use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{Body, RatioData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub segments: Option<u16>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::TEXT,
    theme::TEXT_DIM,
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: Some("payload.label"),
        description: "Optional prefix shown before the LED bar, e.g. `\"VOL\"` → `VOL ▰▰▰▱▱ 60%`. Falls back to `RatioData.label`; omitted when neither is set.",
    },
    OptionSchema {
        name: "segments",
        type_hint: "u16",
        required: false,
        default: Some("5"),
        description: "Number of LED segments. Defaults to the canonical 5-LED silhouette; raise for finer granularity (e.g. 10) when there's room. Values below 1 are clamped to 1.",
    },
    OptionSchema {
        name: "tone",
        type_hint: "\"neutral\" | \"fill\" | \"drain\"",
        required: false,
        default: Some("\"neutral\""),
        description: "How segment colour follows the value. `neutral` is single `theme.text`. `fill` treats the value as how-full (low → status_error, high → status_ok) — right for VU / quota progress. `drain` inverts (high → status_error) — right for usage-style readouts.",
    },
    OptionSchema {
        name: "value_format",
        type_hint: "\"percent\" | \"fraction\" | \"both\"",
        required: false,
        default: Some("\"percent\""),
        description: "Suffix format. `fraction` and `both` require `RatioData.denominator` — otherwise they fall back to `percent`.",
    },
];

const DEFAULT_SEGMENTS: u16 = 5;
const FILLED: &str = "▰";
const EMPTY: &str = "▱";

/// 5-LED segmented bar for `Ratio`. Discrete chunks instead of a continuous fill — the
/// retro-hardware sibling of `gauge_line`. Tone follows `gauge_battery` semantics: neutral
/// (default) for theme-colour, `fill` for low-is-bad, `drain` for high-is-bad.
pub struct GaugeSegmentRenderer;

impl Renderer for GaugeSegmentRenderer {
    fn name(&self) -> &str {
        "gauge_segment"
    }
    fn description(&self) -> &'static str {
        "Discrete LED-style bar for `Ratio`: by default five chunky pip blocks that light up in steps rather than a continuous fill. Pick this over `gauge_line` when you want a retro hardware look or coarser readability at a glance."
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
            render_segment(frame, area, d, opts, theme);
        }
    }
}

fn render_segment(
    frame: &mut Frame,
    area: Rect,
    data: &RatioData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let specific: Options = opts.parse_specific();
    let ratio = data.value.clamp(0.0, 1.0);
    let prefix = resolve_prefix(opts, data);
    let suffix = format_value(ratio, data.denominator, opts.value_format.as_deref());
    let segments = resolve_segment_count(specific.segments);
    let filled = (f64::from(segments) * ratio).round() as u16;
    let fill = tone_color(ratio, opts.tone.as_deref(), theme);
    let row = Rect::new(area.x, area.y + area.height / 2, area.width, 1);
    let line = Line::from(compose_spans(
        &prefix, &suffix, filled, segments, fill, theme,
    ));
    frame.render_widget(Paragraph::new(line), row);
}

fn compose_spans<'a>(
    prefix: &str,
    suffix: &str,
    filled: u16,
    segments: u16,
    fill: ratatui::style::Color,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let mut spans = Vec::with_capacity(5);
    if !prefix.is_empty() {
        spans.push(Span::styled(
            format!("{prefix} "),
            Style::default().fg(theme.text),
        ));
    }
    spans.push(Span::styled(
        FILLED.repeat(filled as usize),
        Style::default().fg(fill),
    ));
    spans.push(Span::styled(
        EMPTY.repeat(segments.saturating_sub(filled) as usize),
        Style::default().fg(theme.text_dim),
    ));
    spans.push(Span::styled(
        format!(" {suffix}"),
        Style::default().fg(theme.text),
    ));
    spans
}

fn resolve_prefix(opts: &RenderOptions, data: &RatioData) -> String {
    opts.label
        .clone()
        .or_else(|| data.label.clone())
        .unwrap_or_default()
}

fn resolve_segment_count(opt: Option<u16>) -> u16 {
    opt.unwrap_or(DEFAULT_SEGMENTS).max(1)
}

fn format_value(ratio: f64, denominator: Option<u64>, mode: Option<&str>) -> String {
    let pct = (ratio * 100.0).round() as u64;
    match (mode.unwrap_or("percent"), denominator) {
        ("fraction", Some(d)) => format!("{} of {d}", numerator(ratio, d)),
        ("both", Some(d)) => format!("{pct}% ({} of {d})", numerator(ratio, d)),
        _ => format!("{pct}%"),
    }
}

fn numerator(ratio: f64, denominator: u64) -> u64 {
    (ratio * denominator as f64).round() as u64
}

fn tone_color(ratio: f64, tone: Option<&str>, theme: &Theme) -> ratatui::style::Color {
    match tone.unwrap_or("neutral") {
        "fill" => level_color(ratio, theme),
        "drain" => level_color(1.0 - ratio, theme),
        _ => theme.text,
    }
}

fn level_color(ratio: f64, theme: &Theme) -> ratatui::style::Color {
    if ratio < 0.20 {
        theme.status_error
    } else if ratio < 0.50 {
        theme.status_warn
    } else {
        theme.status_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, RatioData};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

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

    fn registry_and_spec() -> (Registry, RenderSpec) {
        (
            Registry::with_builtins(),
            RenderSpec::Short("gauge_segment".into()),
        )
    }

    #[test]
    fn renders_default_five_segments() {
        let (registry, spec) = registry_and_spec();
        let buf = render_to_buffer_with_spec(&payload(0.6, None), Some(&spec), &registry, 20, 1);
        let row = line_text(&buf, 0);
        let filled = row.matches(FILLED).count();
        let empty = row.matches(EMPTY).count();
        assert_eq!(filled + empty, DEFAULT_SEGMENTS as usize, "row: {row:?}");
        assert_eq!(filled, 3, "60% of 5 should round to 3 filled: {row:?}");
        assert!(row.contains("60%"), "missing percent suffix: {row:?}");
    }

    #[test]
    fn segments_option_overrides_count() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "gauge_segment", segments = 10 }"#).unwrap();
        let buf =
            render_to_buffer_with_spec(&payload(0.3, None), Some(&w.render), &registry, 30, 1);
        let row = line_text(&buf, 0);
        assert_eq!(row.matches(FILLED).count() + row.matches(EMPTY).count(), 10);
    }

    #[test]
    fn label_prefix_renders_when_provided() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "gauge_segment", label = "VOL" }"#).unwrap();
        let buf =
            render_to_buffer_with_spec(&payload(0.4, None), Some(&w.render), &registry, 30, 1);
        let row = line_text(&buf, 0);
        assert!(row.starts_with("VOL"), "missing prefix: {row:?}");
    }

    #[test]
    fn payload_label_used_when_option_missing() {
        let (registry, spec) = registry_and_spec();
        let buf =
            render_to_buffer_with_spec(&payload(0.5, Some("CPU")), Some(&spec), &registry, 30, 1);
        let row = line_text(&buf, 0);
        assert!(row.starts_with("CPU"), "missing payload label: {row:?}");
    }

    #[test]
    fn clamps_out_of_range() {
        let (registry, spec) = registry_and_spec();
        let _ = render_to_buffer_with_spec(&payload(1.7, None), Some(&spec), &registry, 30, 1);
        let _ = render_to_buffer_with_spec(&payload(-0.2, None), Some(&spec), &registry, 30, 1);
    }

    #[test]
    fn segments_clamped_to_at_least_one() {
        assert_eq!(resolve_segment_count(Some(0)), 1);
        assert_eq!(resolve_segment_count(None), DEFAULT_SEGMENTS);
        assert_eq!(resolve_segment_count(Some(8)), 8);
    }

    #[test]
    fn tone_neutral_is_default_and_uses_text_colour() {
        let theme = Theme::default();
        assert_eq!(tone_color(0.05, None, &theme), theme.text);
        assert_eq!(tone_color(0.95, Some("neutral"), &theme), theme.text);
    }

    #[test]
    fn tone_fill_maps_low_to_error() {
        let theme = Theme::default();
        assert_eq!(tone_color(0.05, Some("fill"), &theme), theme.status_error);
        assert_eq!(tone_color(0.30, Some("fill"), &theme), theme.status_warn);
        assert_eq!(tone_color(0.80, Some("fill"), &theme), theme.status_ok);
    }

    #[test]
    fn tone_drain_inverts_for_usage_metrics() {
        let theme = Theme::default();
        assert_eq!(tone_color(0.95, Some("drain"), &theme), theme.status_error);
        assert_eq!(tone_color(0.70, Some("drain"), &theme), theme.status_warn);
        assert_eq!(tone_color(0.20, Some("drain"), &theme), theme.status_ok);
    }

    #[test]
    fn value_format_fraction_falls_back_without_denominator() {
        assert_eq!(format_value(0.5, None, Some("fraction")), "50%");
        assert_eq!(format_value(0.5, Some(10), Some("fraction")), "5 of 10");
    }

    #[test]
    fn empty_area_does_not_panic() {
        let (registry, spec) = registry_and_spec();
        let _ = render_to_buffer_with_spec(&payload(0.5, None), Some(&spec), &registry, 0, 0);
    }
}
