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

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: Some("payload.label"),
        description: "Prefix shown before the bar, e.g. `\"year\"` → `year: ████░░░ 32%`. Falls back to `RatioData.label` when omitted.",
    },
    OptionSchema {
        name: "value_format",
        type_hint: "\"percent\" | \"fraction\" | \"both\"",
        required: false,
        default: Some("\"percent\""),
        description: "Suffix format. `fraction` and `both` require `RatioData.denominator` — otherwise they fall back to `percent`.",
    },
];

/// Compact progress indicator: a single-line bar, ratio filled. Good for dense dashboards where
/// the full-height `gauge_circle` block renderer is too tall. Alternate renderer for the `Ratio` shape.
pub struct GaugeLineRenderer;

const FILLED: &str = "▓";
const EMPTY: &str = "░";

impl Renderer for GaugeLineRenderer {
    fn name(&self) -> &str {
        "gauge_line"
    }
    fn description(&self) -> &'static str {
        "Single-row progress bar for `Ratio`: optional label, shaded fill across the available width, and a percent or fraction suffix. The lightest member of the `gauge_*` family — pick this for dense dashboards where `gauge_circle` is too tall."
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
            render_line_gauge(frame, area, d, opts, theme);
        }
    }
}

fn render_line_gauge(
    frame: &mut Frame,
    area: Rect,
    data: &RatioData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let ratio = data.value.clamp(0.0, 1.0);
    let prefix = resolve_prefix(opts, data);
    let suffix = format_value(ratio, data.denominator, opts.value_format.as_deref());
    let reserved = prefix_width(&prefix) + 1 + suffix.chars().count() as u16 + 1;
    let bar_width = area.width.saturating_sub(reserved);
    let filled = (f64::from(bar_width) * ratio).round() as u16;
    let line = Line::from(compose_spans(&prefix, &suffix, filled, bar_width, theme));
    frame.render_widget(Paragraph::new(line), area);
}

fn resolve_prefix(opts: &RenderOptions, data: &RatioData) -> String {
    opts.label
        .clone()
        .or_else(|| data.label.clone())
        .unwrap_or_default()
}

fn prefix_width(prefix: &str) -> u16 {
    if prefix.is_empty() {
        0
    } else {
        // "label: " — count chars + suffix ": ".
        prefix.chars().count() as u16 + 2
    }
}

fn compose_spans<'a>(
    prefix: &str,
    suffix: &str,
    filled: u16,
    total: u16,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let mut spans = Vec::with_capacity(5);
    if !prefix.is_empty() {
        spans.push(Span::styled(
            format!("{prefix}: "),
            Style::default().fg(theme.text),
        ));
    }
    spans.push(Span::styled(
        FILLED.repeat(filled as usize),
        Style::default().fg(theme.text),
    ));
    spans.push(Span::styled(
        EMPTY.repeat(total.saturating_sub(filled) as usize),
        Style::default().fg(theme.text_dim),
    ));
    spans.push(Span::styled(
        format!(" {suffix}"),
        Style::default().fg(theme.text),
    ));
    spans
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, RatioData, TextData};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn payload(value: f64) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Ratio(RatioData {
                value,
                label: Some("%".into()),
                denominator: None,
            }),
        }
    }

    fn render_direct(body: &Body) -> Buffer {
        let backend = TestBackend::new(20, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        let registry = Registry::with_builtins();
        terminal
            .draw(|frame| {
                GaugeLineRenderer.render(
                    frame,
                    frame.area(),
                    body,
                    &RenderOptions::default(),
                    &theme,
                    &registry,
                )
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn renders_without_panicking() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("gauge_line".into());
        let _ = render_to_buffer_with_spec(&payload(0.4), Some(&spec), &registry, 20, 3);
    }

    #[test]
    fn clamps_out_of_range() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("gauge_line".into());
        let _ = render_to_buffer_with_spec(&payload(1.7), Some(&spec), &registry, 20, 3);
        let _ = render_to_buffer_with_spec(&payload(-0.2), Some(&spec), &registry, 20, 3);
    }

    #[test]
    fn inline_label_and_percent_suffix_rendered() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "gauge_line", label = "year" }"#).unwrap();
        let buf = render_to_buffer_with_spec(&payload(0.32), Some(&w.render), &registry, 40, 1);
        let row = line_text(&buf, 0);
        assert!(row.starts_with("year:"));
        assert!(row.contains("32%"));
    }

    #[test]
    fn value_format_both_requires_denominator() {
        assert_eq!(
            format_value(0.32, Some(365), Some("both")),
            "32% (117 of 365)"
        );
        assert_eq!(format_value(0.32, None, Some("both")), "32%");
    }

    #[test]
    fn value_format_fraction_falls_back_without_denominator() {
        assert_eq!(format_value(0.50, None, Some("fraction")), "50%");
        assert_eq!(format_value(0.50, Some(10), Some("fraction")), "5 of 10");
    }

    #[test]
    fn renderer_contract_and_helper_defaults_are_stable() {
        let renderer = GaugeLineRenderer;
        assert_eq!(renderer.name(), "gauge_line");
        assert!(renderer.description().contains("Single-row progress bar"));
        assert_eq!(renderer.accepts(), &[Shape::Ratio]);
        assert_eq!(
            renderer
                .color_keys()
                .iter()
                .map(|key| key.name)
                .collect::<Vec<_>>(),
            vec!["text", "text_dim"]
        );
        assert_eq!(
            renderer
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["label", "value_format"]
        );
        assert_eq!(
            resolve_prefix(
                &RenderOptions {
                    label: Some("explicit".into()),
                    ..RenderOptions::default()
                },
                &RatioData {
                    value: 0.0,
                    label: Some("payload".into()),
                    denominator: Some(10),
                },
            ),
            "explicit"
        );
        assert_eq!(
            resolve_prefix(
                &RenderOptions::default(),
                &RatioData {
                    value: 0.0,
                    label: Some("payload".into()),
                    denominator: Some(10),
                },
            ),
            "payload"
        );
        assert_eq!(
            resolve_prefix(
                &RenderOptions::default(),
                &RatioData {
                    value: 0.0,
                    label: None,
                    denominator: None,
                },
            ),
            ""
        );
        assert_eq!(prefix_width(""), 0);
        assert_eq!(prefix_width("abc"), 5);
        assert_eq!(numerator(0.35, 10), 4);
        assert_eq!(format_value(0.35, Some(10), None), "35%");
        assert_eq!(format_value(0.35, Some(10), Some("unexpected")), "35%");
    }

    #[test]
    fn compose_spans_without_prefix_and_wrong_shape_render_noop() {
        let spans = compose_spans("", "50%", 2, 4, &Theme::default());
        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(spans.len(), 3);
        assert_eq!(text, "▓▓░░ 50%");

        let buf = render_direct(&Body::Text(TextData {
            value: "wrong shape".into(),
        }));
        assert_eq!(line_text(&buf, 0).trim(), "");
    }
}
