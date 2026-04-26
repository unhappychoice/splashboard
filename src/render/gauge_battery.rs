use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{Body, RatioData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

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
        description: "Optional prefix shown before the battery, e.g. `\"BAT\"` → `BAT ▕████░▏▮ 75%`. Falls back to `RatioData.label`; omitted when neither is set.",
    },
    OptionSchema {
        name: "tone",
        type_hint: "\"neutral\" | \"fill\" | \"drain\"",
        required: false,
        default: Some("\"neutral\""),
        description: "How the fill colour follows the value. `neutral` is single `theme.text` (matches the rest of the `gauge_*` family). `fill` treats the value as how-full (low → status_error, high → status_ok) — right for battery / quota progress. `drain` inverts (high → status_error) — right for `system_disk_usage` / `system_memory` / `system_cpu` where the ratio is \"fraction used\".",
    },
];

/// Battery-icon renderer for `Ratio`. Compact on short slots, boxed on tall slots. The fill
/// colour is theme-neutral by default; opt into a level-driven palette via `tone = "fill"` for
/// battery-style readouts (low → red) or `tone = "drain"` for usage-style readouts (high → red).
pub struct GaugeBatteryRenderer;

const FILLED: &str = "█";
const EMPTY: &str = "░";

impl Renderer for GaugeBatteryRenderer {
    fn name(&self) -> &str {
        "gauge_battery"
    }
    fn description(&self) -> &'static str {
        "Battery silhouette for `Ratio`: framed cell with internal fill bars, a tip cap on the right, and a percent label. Renders as a single-line pill in short slots and as a boxed three-row icon when the slot is tall enough."
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
            render_battery(frame, area, d, opts, theme);
        }
    }
}

fn render_battery(
    frame: &mut Frame,
    area: Rect,
    data: &RatioData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let ratio = data.value.clamp(0.0, 1.0);
    let prefix = resolve_prefix(opts, data);
    let percent = format!("{}%", (ratio * 100.0).round() as u64);
    let fill = tone_color(ratio, opts.tone.as_deref(), theme);
    if area.height < 3 {
        render_compact(frame, area, ratio, &prefix, &percent, fill, theme);
    } else {
        render_boxed(frame, area, ratio, &prefix, &percent, fill, theme);
    }
}

fn render_compact(
    frame: &mut Frame,
    area: Rect,
    ratio: f64,
    prefix: &str,
    percent: &str,
    fill: Color,
    theme: &Theme,
) {
    let prefix_w = prefix_width(prefix);
    let suffix_w = percent.chars().count() as u16 + 1;
    let frame_w = 2u16; // "▕" + "▏"
    let tip_w = 1u16; // "▮"
    let cells = area
        .width
        .saturating_sub(prefix_w + suffix_w + frame_w + tip_w);
    let filled = (f64::from(cells) * ratio).round() as u16;
    let mut spans = Vec::with_capacity(8);
    if !prefix.is_empty() {
        spans.push(Span::styled(
            format!("{prefix} "),
            Style::default().fg(theme.text),
        ));
    }
    spans.push(Span::styled("▕", Style::default().fg(theme.text)));
    spans.push(Span::styled(
        FILLED.repeat(filled as usize),
        Style::default().fg(fill),
    ));
    spans.push(Span::styled(
        EMPTY.repeat(cells.saturating_sub(filled) as usize),
        Style::default().fg(theme.text_dim),
    ));
    spans.push(Span::styled("▏", Style::default().fg(theme.text)));
    spans.push(Span::styled("▮", Style::default().fg(theme.text)));
    spans.push(Span::styled(
        format!(" {percent}"),
        Style::default().fg(theme.text),
    ));
    let mid_y = area.y + area.height / 2;
    let row = Rect::new(area.x, mid_y, area.width, 1);
    frame.render_widget(Paragraph::new(Line::from(spans)), row);
}

fn render_boxed(
    frame: &mut Frame,
    area: Rect,
    ratio: f64,
    prefix: &str,
    percent: &str,
    fill: Color,
    theme: &Theme,
) {
    let prefix_w = prefix_width(prefix);
    let suffix = format!(" {percent}");
    let suffix_w = suffix.chars().count() as u16;
    let tip_w = 1u16;
    let body_w = area
        .width
        .saturating_sub(prefix_w + suffix_w + tip_w)
        .max(3);
    let inner_w = body_w.saturating_sub(2); // borders
    let filled_cells = (f64::from(inner_w) * ratio).round() as u16;
    let body_h = area.height.min(3);
    let top_y = area.y + (area.height - body_h) / 2;
    let body_x = area.x + prefix_w;

    if !prefix.is_empty() {
        let label = Rect::new(area.x, top_y + body_h / 2, prefix_w, 1);
        frame.render_widget(
            Paragraph::new(prefix.to_string()).style(Style::default().fg(theme.text)),
            label,
        );
    }

    let buf = frame.buffer_mut();
    paint_horizontal(buf, body_x, top_y, inner_w, "┌", "┐", theme.text);
    paint_horizontal(
        buf,
        body_x,
        top_y + body_h - 1,
        inner_w,
        "└",
        "┘",
        theme.text,
    );
    paint_sides(
        buf,
        body_x,
        top_y + 1,
        inner_w,
        body_h.saturating_sub(2),
        theme.text,
    );
    let fill_rect = Rect::new(body_x + 1, top_y + 1, inner_w, body_h.saturating_sub(2));
    paint_fill(buf, fill_rect, filled_cells, fill, theme.text_dim);
    paint_tip(buf, body_x + body_w, top_y, body_h, theme.text);

    let suffix_rect = Rect::new(body_x + body_w + tip_w, top_y + body_h / 2, suffix_w, 1);
    frame.render_widget(
        Paragraph::new(suffix).style(Style::default().fg(theme.text)),
        suffix_rect,
    );
}

fn paint_horizontal(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    inner_w: u16,
    left: &str,
    right: &str,
    color: Color,
) {
    set_cell(buf, x, y, left, color);
    (0..inner_w).for_each(|i| set_cell(buf, x + 1 + i, y, "─", color));
    set_cell(buf, x + 1 + inner_w, y, right, color);
}

fn paint_sides(buf: &mut Buffer, x: u16, y: u16, inner_w: u16, h: u16, color: Color) {
    (0..h).for_each(|j| {
        set_cell(buf, x, y + j, "│", color);
        set_cell(buf, x + 1 + inner_w, y + j, "│", color);
    });
}

fn paint_fill(buf: &mut Buffer, rect: Rect, filled: u16, fill: Color, empty: Color) {
    (0..rect.height).for_each(|j| {
        (0..rect.width).for_each(|i| {
            let (glyph, color) = if i < filled {
                (FILLED, fill)
            } else {
                (EMPTY, empty)
            };
            set_cell(buf, rect.x + i, rect.y + j, glyph, color);
        });
    });
}

fn paint_tip(buf: &mut Buffer, x: u16, top_y: u16, body_h: u16, color: Color) {
    let mid = top_y + body_h / 2;
    set_cell(buf, x, mid, "▮", color);
}

fn set_cell(buf: &mut Buffer, x: u16, y: u16, glyph: &str, color: Color) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(glyph);
        cell.set_style(Style::default().fg(color));
    }
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
        prefix.chars().count() as u16 + 1
    }
}

fn tone_color(ratio: f64, tone: Option<&str>, theme: &Theme) -> Color {
    match tone.unwrap_or("neutral") {
        "fill" => level_color(ratio, theme),
        "drain" => level_color(1.0 - ratio, theme),
        _ => theme.text,
    }
}

fn level_color(ratio: f64, theme: &Theme) -> Color {
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
            RenderSpec::Short("gauge_battery".into()),
        )
    }

    #[test]
    fn renders_compact_in_short_slot() {
        let (registry, spec) = registry_and_spec();
        let buf = render_to_buffer_with_spec(&payload(0.75, None), Some(&spec), &registry, 30, 1);
        let row = line_text(&buf, 0);
        assert!(row.contains("▕"), "missing left frame: {row:?}");
        assert!(row.contains("▏"), "missing right frame: {row:?}");
        assert!(row.contains("▮"), "missing tip: {row:?}");
        assert!(row.contains("75%"), "missing percent: {row:?}");
    }

    #[test]
    fn renders_boxed_in_tall_slot() {
        let (registry, spec) = registry_and_spec();
        let buf = render_to_buffer_with_spec(&payload(0.5, None), Some(&spec), &registry, 30, 5);
        let joined: String = (0..5).map(|y| line_text(&buf, y)).collect();
        assert!(joined.contains("┌"), "missing top-left corner: {joined:?}");
        assert!(
            joined.contains("┘"),
            "missing bottom-right corner: {joined:?}"
        );
        assert!(joined.contains("▮"), "missing tip: {joined:?}");
        assert!(joined.contains("50%"), "missing percent: {joined:?}");
    }

    #[test]
    fn clamps_out_of_range() {
        let (registry, spec) = registry_and_spec();
        let _ = render_to_buffer_with_spec(&payload(1.7, None), Some(&spec), &registry, 30, 5);
        let _ = render_to_buffer_with_spec(&payload(-0.2, None), Some(&spec), &registry, 30, 5);
    }

    #[test]
    fn label_prefix_renders_when_provided() {
        let (registry, _) = registry_and_spec();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "gauge_battery", label = "BAT" }"#).unwrap();
        let buf =
            render_to_buffer_with_spec(&payload(0.4, None), Some(&w.render), &registry, 40, 1);
        let row = line_text(&buf, 0);
        assert!(row.starts_with("BAT"), "missing prefix: {row:?}");
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
    fn unknown_tone_falls_back_to_neutral() {
        let theme = Theme::default();
        assert_eq!(tone_color(0.05, Some("garbage"), &theme), theme.text);
    }

    #[test]
    fn empty_area_does_not_panic() {
        let (registry, spec) = registry_and_spec();
        let _ = render_to_buffer_with_spec(&payload(0.5, None), Some(&spec), &registry, 0, 0);
    }
}
