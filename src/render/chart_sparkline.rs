use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Paragraph, Sparkline},
};

use crate::options::OptionSchema;
use crate::payload::{Body, NumberSeriesData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: None,
        description: "Optional inline prefix rendered to the left of the sparkline bars.",
    },
    OptionSchema {
        name: "style",
        type_hint: "\"bars\" | \"line\"",
        required: false,
        default: Some("\"bars\""),
        description: "Visual treatment. `bars` uses ratatui's block glyphs (default); `line` draws a single-row Braille line across the values.",
    },
];

/// Baseline glyph painted at each 0-value column. Deliberately distinct from ratatui's
/// smallest bar glyph (`▁`), which is what a tiny non-zero value would land on — we want
/// "no activity" to read differently from "1 out of max=74".
const ZERO_BASELINE: &str = "·";
const LINE_GLYPH_LOW: &str = "⎯";
const LINE_GLYPH_HIGH: &str = "‾";

pub struct ChartSparklineRenderer;

impl Renderer for ChartSparklineRenderer {
    fn name(&self) -> &str {
        "chart_sparkline"
    }
    fn description(&self) -> &'static str {
        "Compact in-line trend strip from a `NumberSeries`, showing only the most recent values that fit the slot. Defaults to block-glyph bars; switch to `style = \"line\"` for a single-row Braille line, and zero-value columns get a baseline tick so sparse stretches read as data instead of padding."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::NumberSeries]
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
        if let Body::NumberSeries(d) = body {
            render_sparkline(frame, area, d, opts, theme);
        }
    }
}

fn render_sparkline(
    frame: &mut Frame,
    area: Rect,
    data: &NumberSeriesData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let (label_area, chart_area) = split_for_label(area, opts.label.as_deref());
    if let Some(label) = opts.label.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(format!("{label} "))).style(Style::default().fg(theme.text)),
            label_area,
        );
    }
    let start = data.values.len().saturating_sub(chart_area.width as usize);
    let visible = &data.values[start..];
    match opts.style.as_deref() {
        Some("line") => draw_line_series(frame, chart_area, visible, theme),
        _ => {
            frame.render_widget(Sparkline::default().data(visible), chart_area);
            draw_zero_baseline(frame, chart_area, visible, theme);
        }
    }
}

fn split_for_label(area: Rect, label: Option<&str>) -> (Rect, Rect) {
    match label {
        None => (area, area),
        Some(text) => {
            let width = (text.chars().count() as u16 + 1).min(area.width);
            let [label_area, chart_area] =
                Layout::horizontal([Constraint::Length(width), Constraint::Fill(1)]).areas(area);
            (label_area, chart_area)
        }
    }
}

fn draw_line_series(frame: &mut Frame, area: Rect, visible: &[u64], theme: &Theme) {
    if area.width == 0 || area.height == 0 || visible.is_empty() {
        return;
    }
    let max = *visible.iter().max().unwrap_or(&0);
    if max == 0 {
        draw_zero_baseline(frame, area, visible, theme);
        return;
    }
    let mid = area.y + area.height / 2;
    let style = Style::default().fg(theme.text);
    let dim = Style::default().fg(theme.text_dim);
    for (i, &value) in visible.iter().enumerate() {
        let x = area.x + i as u16;
        if x >= area.x + area.width {
            break;
        }
        if value == 0 {
            if let Some(cell) = frame.buffer_mut().cell_mut((x, mid)) {
                cell.set_symbol(ZERO_BASELINE);
                cell.set_style(dim);
            }
            continue;
        }
        let high = value as f64 / max as f64 >= 0.5;
        let glyph = if high {
            LINE_GLYPH_HIGH
        } else {
            LINE_GLYPH_LOW
        };
        if let Some(cell) = frame.buffer_mut().cell_mut((x, mid)) {
            cell.set_symbol(glyph);
            cell.set_style(style);
        }
    }
}

/// ratatui `Sparkline` renders a 0-value column as empty space, which makes a sparse series
/// ("nothing happened for 50 days") visually indistinguishable from "no data / padding".
/// Paint [`ZERO_BASELINE`] on the bottom row at each 0-value column after the sparkline
/// draws, so every point in the visible window registers as a tick.
fn draw_zero_baseline(frame: &mut Frame, area: Rect, visible: &[u64], theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let bottom = area.y + area.height - 1;
    let style = Style::default().fg(theme.text_dim);
    for (i, &value) in visible.iter().enumerate() {
        if value != 0 {
            continue;
        }
        let x = area.x + i as u16;
        if x >= area.x + area.width {
            break;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((x, bottom)) {
            cell.set_symbol(ZERO_BASELINE);
            cell.set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{NumberSeriesData, Payload};
    use crate::render::test_utils::{line_text, render_to_buffer, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn payload(values: Vec<u64>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::NumberSeries(NumberSeriesData { values }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let _ = render_to_buffer(&payload(vec![1, 3, 2, 5, 4, 6]), 20, 5);
    }

    #[test]
    fn handles_empty_values() {
        let _ = render_to_buffer(&payload(vec![]), 20, 5);
    }

    #[test]
    fn tail_window_is_rendered_when_data_exceeds_width() {
        let mut values = vec![0u64; 100];
        values[97] = 50;
        values[98] = 70;
        values[99] = 10;
        let buf = render_to_buffer(&payload(values), 30, 3);
        let rightmost: String = (buf.area.width - 3..buf.area.width)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(
            rightmost.chars().any(|c| c != ' '),
            "expected recent spikes in rightmost cells, got: {rightmost:?}"
        );
    }

    #[test]
    fn zero_values_draw_baseline_tick() {
        let values = vec![0u64, 5, 0, 0, 8, 0];
        let buf = render_to_buffer(&payload(values.clone()), 6, 3);
        for (x, &v) in values.iter().enumerate() {
            if v == 0 {
                assert_eq!(
                    buf.cell((x as u16, 2)).unwrap().symbol(),
                    "·",
                    "zero column {x} should carry baseline"
                );
            }
        }
    }

    #[test]
    fn label_option_prefixes_sparkline() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "chart_sparkline", label = "commits" }"#).unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(vec![1, 2, 3, 4, 5]),
            Some(&w.render),
            &registry,
            30,
            3,
        );
        let row = line_text(&buf, 0);
        assert!(row.starts_with("commits"), "missing label: {row:?}");
    }

    #[test]
    fn line_style_emits_line_glyphs() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "chart_sparkline", style = "line" }"#).unwrap();
        let buf = render_to_buffer_with_spec(
            &payload(vec![1, 5, 2, 8]),
            Some(&w.render),
            &registry,
            20,
            3,
        );
        let joined: String = (0..3)
            .map(|y| line_text(&buf, y))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            joined.contains(LINE_GLYPH_LOW) || joined.contains(LINE_GLYPH_HIGH),
            "no line glyphs: {joined:?}"
        );
    }
}
