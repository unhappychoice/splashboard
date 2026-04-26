use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
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
        description: "Optional prefix shown left of the tube on the mid row, e.g. `\"CPU\"` → `CPU ╭─╮ 32%`. Falls back to `RatioData.label`; omitted when neither is set.",
    },
    OptionSchema {
        name: "tone",
        type_hint: "\"neutral\" | \"fill\" | \"drain\"",
        required: false,
        default: Some("\"neutral\""),
        description: "How the mercury colour follows the value. `neutral` is single `theme.text` (matches the rest of the `gauge_*` family). `fill` treats value as how-full (low → status_error, high → status_ok) — right for quota / progress. `drain` inverts (high → status_error) — right for `system_cpu` / `system_memory` / `disk_usage` where the ratio is \"fraction used\".",
    },
    OptionSchema {
        name: "value_format",
        type_hint: "\"percent\" | \"fraction\" | \"both\"",
        required: false,
        default: Some("\"percent\""),
        description: "Suffix format. `fraction` and `both` require `RatioData.denominator` — otherwise they fall back to `percent`.",
    },
];

const TUBE_WIDTH: u16 = 3;
const FILLED: &str = "█";
const EMPTY: &str = "░";
const BULB: &str = "●";

/// Vertical mercury-tube renderer for `Ratio`. Mercury rises from a bulb at the bottom of a
/// glass column — the vertical sibling of the horizontal `gauge_line`. Tone follows the
/// `gauge_battery` / `gauge_segment` semantics: theme-neutral by default; opt into a
/// level-driven palette via `tone = "fill"` (low → red) or `tone = "drain"` (high → red).
pub struct GaugeThermometerRenderer;

impl Renderer for GaugeThermometerRenderer {
    fn name(&self) -> &str {
        "gauge_thermometer"
    }
    fn description(&self) -> &'static str {
        "Vertical mercury tube for `Ratio`: rounded glass walls with a bulb at the bottom and a mercury column rising from it, with the label and percent on the mid row. The vertical sibling of `gauge_line` — pick this for tall, narrow slots where height carries the meaning."
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
            render_thermometer(frame, area, d, opts, theme);
        }
    }
}

fn render_thermometer(
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
    let suffix = format!(
        " {}",
        format_value(ratio, data.denominator, opts.value_format.as_deref())
    );
    let fill = tone_color(ratio, opts.tone.as_deref(), theme);
    let prefix_w = label_slot(&prefix);
    let suffix_w = suffix.chars().count() as u16;
    let walled = area.height >= 2 && area.width >= prefix_w + TUBE_WIDTH + suffix_w;
    if walled {
        render_walled(frame, area, ratio, &prefix, &suffix, fill, theme);
    } else {
        render_fill_column(frame, area, ratio, fill, theme);
    }
}

fn render_walled(
    frame: &mut Frame,
    area: Rect,
    ratio: f64,
    prefix: &str,
    suffix: &str,
    fill: Color,
    theme: &Theme,
) {
    let tube_x = area.x + label_slot(prefix);
    let mid_y = area.y + area.height / 2;
    if !prefix.is_empty() {
        paint_text(frame, area.x, mid_y, prefix, theme.text);
    }
    paint_text(frame, tube_x + TUBE_WIDTH, mid_y, suffix, theme.text);
    let buf = frame.buffer_mut();
    paint_top_cap(buf, tube_x, area.y, theme.text);
    paint_bulb(buf, tube_x, area.y + area.height - 1, fill, theme.text);
    let interior = area.height.saturating_sub(2);
    paint_interior(buf, tube_x, area.y + 1, interior, ratio, fill, theme);
}

fn paint_top_cap(buf: &mut Buffer, x: u16, y: u16, color: Color) {
    set_cell(buf, x, y, "╭", color);
    set_cell(buf, x + 1, y, "─", color);
    set_cell(buf, x + 2, y, "╮", color);
}

fn paint_bulb(buf: &mut Buffer, x: u16, y: u16, fill: Color, frame_color: Color) {
    set_cell(buf, x, y, "╰", frame_color);
    set_cell(buf, x + 1, y, BULB, fill);
    set_cell(buf, x + 2, y, "╯", frame_color);
}

fn paint_interior(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    h: u16,
    ratio: f64,
    fill: Color,
    theme: &Theme,
) {
    let filled = (f64::from(h) * ratio).round() as u16;
    (0..h).for_each(|i| paint_interior_row(buf, x, y + i, h - 1 - i, filled, fill, theme));
}

fn paint_interior_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    from_bottom: u16,
    filled: u16,
    fill: Color,
    theme: &Theme,
) {
    let is_filled = from_bottom < filled;
    let (glyph, color) = if is_filled {
        (FILLED, fill)
    } else {
        (EMPTY, theme.text_dim)
    };
    set_cell(buf, x, y, "│", theme.text);
    set_cell(buf, x + 1, y, glyph, color);
    set_cell(buf, x + 2, y, "│", theme.text);
}

fn render_fill_column(frame: &mut Frame, area: Rect, ratio: f64, fill: Color, theme: &Theme) {
    let filled = (f64::from(area.height) * ratio).round() as u16;
    let buf = frame.buffer_mut();
    (0..area.height).for_each(|i| {
        let from_bottom = area.height - 1 - i;
        let is_filled = from_bottom < filled;
        let (glyph, color) = if is_filled {
            (FILLED, fill)
        } else {
            (EMPTY, theme.text_dim)
        };
        (0..area.width).for_each(|dx| set_cell(buf, area.x + dx, area.y + i, glyph, color));
    });
}

fn paint_text(frame: &mut Frame, x: u16, y: u16, text: &str, color: Color) {
    let rect = Rect::new(x, y, text.chars().count() as u16, 1);
    frame.render_widget(
        Paragraph::new(text.to_string()).style(Style::default().fg(color)),
        rect,
    );
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

fn label_slot(prefix: &str) -> u16 {
    if prefix.is_empty() {
        0
    } else {
        prefix.chars().count() as u16 + 1
    }
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
            RenderSpec::Short("gauge_thermometer".into()),
        )
    }

    fn joined(buf: &ratatui::buffer::Buffer) -> String {
        (0..buf.area.height).map(|y| line_text(buf, y)).collect()
    }

    #[test]
    fn renders_walled_tube_in_tall_slot() {
        let (registry, spec) = registry_and_spec();
        let buf = render_to_buffer_with_spec(&payload(0.5, None), Some(&spec), &registry, 12, 8);
        let all = joined(&buf);
        assert!(all.contains("╭"), "missing top-left cap: {all:?}");
        assert!(all.contains("╮"), "missing top-right cap: {all:?}");
        assert!(all.contains("╰"), "missing bottom-left bulb frame: {all:?}");
        assert!(
            all.contains("╯"),
            "missing bottom-right bulb frame: {all:?}"
        );
        assert!(all.contains(BULB), "missing bulb glyph: {all:?}");
        assert!(all.contains(FILLED), "missing mercury fill: {all:?}");
        assert!(all.contains(EMPTY), "missing empty interior: {all:?}");
        assert!(all.contains("50%"), "missing percent suffix: {all:?}");
    }

    #[test]
    fn mercury_grows_from_bottom_with_ratio() {
        let (registry, spec) = registry_and_spec();
        let full = render_to_buffer_with_spec(&payload(1.0, None), Some(&spec), &registry, 12, 8);
        let empty = render_to_buffer_with_spec(&payload(0.0, None), Some(&spec), &registry, 12, 8);
        let full_str = joined(&full);
        let empty_str = joined(&empty);
        assert!(
            full_str.matches(FILLED).count() > empty_str.matches(FILLED).count(),
            "ratio=1.0 should fill more than ratio=0.0"
        );
        assert_eq!(
            empty_str.matches(FILLED).count(),
            0,
            "ratio=0.0 leaves the interior empty: {empty_str:?}"
        );
    }

    #[test]
    fn narrow_width_falls_back_to_fill_column() {
        let (registry, spec) = registry_and_spec();
        let buf = render_to_buffer_with_spec(&payload(0.5, None), Some(&spec), &registry, 1, 6);
        let all = joined(&buf);
        assert!(
            !all.contains("╭") && !all.contains("╯"),
            "narrow slot should not draw walls: {all:?}"
        );
        assert!(
            all.contains(FILLED) && all.contains(EMPTY),
            "narrow slot still shows fill / empty cells: {all:?}"
        );
    }

    #[test]
    fn height_two_renders_top_cap_and_bulb_only() {
        let (registry, spec) = registry_and_spec();
        let buf = render_to_buffer_with_spec(&payload(0.7, None), Some(&spec), &registry, 12, 2);
        let all = joined(&buf);
        assert!(all.contains("╭"), "top cap row missing: {all:?}");
        assert!(all.contains("╰"), "bulb row missing: {all:?}");
        assert!(all.contains(BULB), "bulb glyph missing: {all:?}");
        assert!(all.contains("70%"), "percent suffix missing: {all:?}");
    }

    #[test]
    fn label_prefix_renders_when_provided() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "gauge_thermometer", label = "CPU" }"#).unwrap();
        let buf =
            render_to_buffer_with_spec(&payload(0.4, None), Some(&w.render), &registry, 16, 6);
        let all = joined(&buf);
        assert!(all.contains("CPU"), "missing label prefix: {all:?}");
    }

    #[test]
    fn payload_label_used_when_option_missing() {
        let (registry, spec) = registry_and_spec();
        let buf =
            render_to_buffer_with_spec(&payload(0.5, Some("MEM")), Some(&spec), &registry, 16, 6);
        let all = joined(&buf);
        assert!(all.contains("MEM"), "payload label not rendered: {all:?}");
    }

    #[test]
    fn clamps_out_of_range() {
        let (registry, spec) = registry_and_spec();
        let _ = render_to_buffer_with_spec(&payload(1.7, None), Some(&spec), &registry, 12, 8);
        let _ = render_to_buffer_with_spec(&payload(-0.2, None), Some(&spec), &registry, 12, 8);
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
    fn value_format_fraction_falls_back_without_denominator() {
        assert_eq!(format_value(0.5, None, Some("fraction")), "50%");
        assert_eq!(format_value(0.5, Some(10), Some("fraction")), "5 of 10");
    }

    #[test]
    fn value_format_both_includes_fraction_when_denominator_present() {
        assert_eq!(
            format_value(0.32, Some(365), Some("both")),
            "32% (117 of 365)"
        );
    }

    #[test]
    fn empty_area_does_not_panic() {
        let (registry, spec) = registry_and_spec();
        let _ = render_to_buffer_with_spec(&payload(0.5, None), Some(&spec), &registry, 0, 0);
    }
}
