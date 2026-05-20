use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{BarsData, Body};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "max_items",
        type_hint: "positive integer",
        required: false,
        default: Some("all bars"),
        description: "Cap on rendered ranks. Keeps the top N by value when the input has more bars than the cap.",
    },
    OptionSchema {
        name: "style",
        type_hint: "\"number\" | \"medal\" | \"none\"",
        required: false,
        default: Some("\"number\""),
        description: "Rank-prefix glyph. `number` (default) emits `1.` / `2.` / `3.`. `medal` emits 🥇 / 🥈 / 🥉 for the top three then numbers for the rest. `none` omits the prefix.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the whole ranking within its cell.",
    },
];

const MEDALS: [&str; 3] = ["🥇", "🥈", "🥉"];
const COLUMN_GAP: &str = "  ";

/// Rank-prefixed list view for the `Bars` shape. Preserves the fetcher's bar order — sort is
/// the fetcher's responsibility, not the renderer's, so that fetchers emitting recency-sorted
/// (`steam_owned_games sort = "recent"`), alphabetical, or any non-value-desc order display as
/// intended. Use `style = "medal"` to highlight the top three with 🥇/🥈/🥉.
pub struct ListRankingRenderer;

impl Renderer for ListRankingRenderer {
    fn name(&self) -> &str {
        "list_ranking"
    }
    fn description(&self) -> &'static str {
        "Rank-prefixed table preserving the fetcher's bar order: rank prefix (`1.` / `2.` numbers, 🥇/🥈/🥉 medals, or none), label, and right-aligned value column. The text-first sibling of `chart_bar` — pick this when the values matter more than their relative bar lengths. Sort is the fetcher's responsibility; `max_items` truncates from the head."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Bars]
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
        if let Body::Bars(d) = body {
            render_ranking(frame, area, d, opts, theme);
        }
    }
}

fn render_ranking(
    frame: &mut Frame,
    area: Rect,
    data: &BarsData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = capped_rows(data, opts.max_items);
    let style = opts.style.as_deref().unwrap_or("number");
    let prefixes: Vec<String> = (0..rows.len()).map(|i| rank_prefix(i, style)).collect();
    let widths = column_widths(&prefixes, &rows);
    // Stretch the label column so the value sits flush against the right edge of
    // the cell instead of leaving dead space, and truncate over-long labels so the
    // value doesn't get clipped off the right.
    let target = align_rect(area, widths.total() as u16, opts.align.as_deref());
    let prefix_part = if widths.prefix > 0 {
        widths.prefix + 1
    } else {
        0
    };
    let fixed = prefix_part + COLUMN_GAP.len() + widths.value;
    let label_width = (target.width as usize).saturating_sub(fixed);
    let effective = Widths {
        prefix: widths.prefix,
        label: label_width,
        value: widths.value,
    };
    let lines: Vec<Line> = rows
        .iter()
        .zip(prefixes.iter())
        .map(|(row, prefix)| compose_line(prefix, &row.label, &row.display, effective, theme))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text)),
        target,
    );
}

/// Row view. `display` is what the right column actually prints — `Bar.value_label` when
/// present, otherwise the numeric value stringified.
struct Row {
    label: String,
    display: String,
}

/// Build rows in the fetcher's bar order. `max_items` truncates from the head — sort is the
/// fetcher's job (lastfm's `chart.getTop*` is server-ranked, steam's `_owned_games` sorts by
/// playtime or recency per its `sort` option, basic_bars carries the user-authored order).
fn capped_rows(data: &BarsData, cap: Option<usize>) -> Vec<Row> {
    let mut rows: Vec<Row> = data
        .bars
        .iter()
        .map(|b| Row {
            label: b.label.clone(),
            display: b.value_label.clone().unwrap_or_else(|| b.value.to_string()),
        })
        .collect();
    if let Some(n) = cap {
        rows.truncate(n);
    }
    rows
}

fn rank_prefix(index: usize, style: &str) -> String {
    match style {
        "none" => String::new(),
        "medal" => MEDALS
            .get(index)
            .map(|m| (*m).to_string())
            .unwrap_or_else(|| format!("{}.", index + 1)),
        _ => format!("{}.", index + 1),
    }
}

#[derive(Debug, Clone, Copy)]
struct Widths {
    prefix: usize,
    label: usize,
    value: usize,
}

impl Widths {
    fn total(self) -> usize {
        let prefix_part = if self.prefix > 0 { self.prefix + 1 } else { 0 };
        prefix_part + self.label + COLUMN_GAP.len() + self.value
    }
}

fn column_widths(prefixes: &[String], rows: &[Row]) -> Widths {
    Widths {
        prefix: prefixes.iter().map(|p| display_width(p)).max().unwrap_or(0),
        label: rows
            .iter()
            .map(|r| r.label.chars().count())
            .max()
            .unwrap_or(0),
        value: rows
            .iter()
            .map(|r| r.display.chars().count())
            .max()
            .unwrap_or(0),
    }
}

fn compose_line<'a>(
    prefix: &str,
    label: &str,
    value: &str,
    widths: Widths,
    theme: &Theme,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(4);
    if widths.prefix > 0 {
        let pad = widths.prefix.saturating_sub(display_width(prefix));
        spans.push(Span::styled(
            format!("{}{prefix} ", " ".repeat(pad)),
            Style::default().fg(theme.text_dim),
        ));
    }
    spans.push(Span::raw(fit_label(label, widths.label)));
    spans.push(Span::raw(COLUMN_GAP));
    spans.push(Span::raw(pad_left(value, widths.value)));
    Line::from(spans)
}

/// Pad with trailing spaces if `label` fits, else truncate from the right (with a single
/// `…` ellipsis) so the value column stays visible. Empty width returns empty.
fn fit_label(label: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let n = label.chars().count();
    if n <= width {
        let mut out = String::with_capacity(width);
        out.push_str(label);
        out.extend(std::iter::repeat_n(' ', width - n));
        return out;
    }
    if width == 1 {
        return "…".to_string();
    }
    let take = width - 1;
    let mut out: String = label.chars().take(take).collect();
    out.push('…');
    out
}

fn align_rect(area: Rect, content_width: u16, align: Option<&str>) -> Rect {
    if content_width == 0 || content_width >= area.width {
        return area;
    }
    let offset = match align {
        Some("center") => (area.width - content_width) / 2,
        Some("right") => area.width - content_width,
        _ => return area,
    };
    Rect {
        x: area.x + offset,
        y: area.y,
        width: content_width,
        height: area.height,
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(width);
    out.extend(std::iter::repeat_n(' ', width - n));
    out.push_str(s);
    out
}

// Medal codepoints render two cells in every wcwidth-aware terminal; everything else we emit
// here is ASCII (digits, dot, padding spaces). A lookup table keeps the column maths honest
// without pulling in a unicode-width dependency for one renderer.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            if matches!(c, '🥇' | '🥈' | '🥉') {
                2
            } else {
                1
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Bar, BarsData, Body, Payload};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn payload(bars: &[(&str, u64)]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Bars(BarsData {
                bars: bars
                    .iter()
                    .map(|(l, v)| Bar {
                        label: (*l).into(),
                        value: *v,
                        value_label: None,
                    })
                    .collect(),
            }),
        }
    }

    fn render_with(spec: &RenderSpec, p: &Payload, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let registry = Registry::with_builtins();
        render_to_buffer_with_spec(p, Some(spec), &registry, w, h)
    }

    #[test]
    fn preserves_fetcher_bar_order_with_numeric_prefix() {
        // The fetcher owns sort order — renderer just rank-prefixes the bars as they arrive.
        // A recency-sorted owned_games list, a server-ranked lastfm chart, and a user-authored
        // basic_bars list all show in their respective intended order.
        let p = payload(&[("alice", 3), ("bob", 7), ("carol", 5)]);
        let spec = RenderSpec::Short("list_ranking".into());
        let buf = render_with(&spec, &p, 30, 3);
        let row0 = line_text(&buf, 0);
        let row1 = line_text(&buf, 1);
        let row2 = line_text(&buf, 2);
        assert!(
            row0.contains("1.") && row0.contains("alice"),
            "row0: {row0:?}"
        );
        assert!(
            row1.contains("2.") && row1.contains("bob"),
            "row1: {row1:?}"
        );
        assert!(
            row2.contains("3.") && row2.contains("carol"),
            "row2: {row2:?}"
        );
    }

    #[test]
    fn max_items_truncates_from_the_head_preserving_order() {
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "list_ranking", max_items = 2 }"#).unwrap();
        // With auto-sort gone, max_items keeps the first 2 in the fetcher's order — not the
        // numerically top 2. A fetcher that wants "top N by value" sorts itself.
        let p = payload(&[("a", 1), ("b", 9), ("c", 5), ("d", 7)]);
        let buf = render_with(&w.render, &p, 30, 4);
        let joined: String = (0..4).map(|y| line_text(&buf, y)).collect();
        assert!(joined.contains("a"), "missing first: {joined:?}");
        assert!(joined.contains("b"), "missing second: {joined:?}");
        assert!(!joined.contains("c"), "third should be capped: {joined:?}");
        assert!(!joined.contains("d"), "fourth should be capped: {joined:?}");
    }

    #[test]
    fn medal_style_decorates_top_three() {
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "list_ranking", style = "medal" }"#).unwrap();
        let p = payload(&[("a", 9), ("b", 7), ("c", 5), ("d", 3)]);
        let buf = render_with(&w.render, &p, 30, 4);
        let r0 = line_text(&buf, 0);
        let r3 = line_text(&buf, 3);
        assert!(r0.contains("🥇"), "row0: {r0:?}");
        assert!(line_text(&buf, 1).contains("🥈"));
        assert!(line_text(&buf, 2).contains("🥉"));
        // Fourth place falls back to a numeric prefix.
        assert!(r3.contains("4."), "row3: {r3:?}");
    }

    #[test]
    fn value_label_overrides_numeric_value_when_present() {
        // Fetchers pre-format a unit ("80h", "2024-07-03") in `Bar.value_label`. The right
        // column prints that text instead of stringifying `value`, so a recent-launches widget
        // shows dates rather than unix timestamps and a steam top-games widget shows hours
        // rather than raw minutes.
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "Game A".into(),
                        value: 4830,
                        value_label: Some("80h".into()),
                    },
                    Bar {
                        label: "Game B".into(),
                        value: 12_000,
                        value_label: Some("200h".into()),
                    },
                ],
            }),
        };
        let spec = RenderSpec::Short("list_ranking".into());
        let buf = render_with(&spec, &payload, 30, 2);
        let row0 = line_text(&buf, 0);
        let row1 = line_text(&buf, 1);
        assert!(row0.contains("Game A") && row0.contains("80h"), "{row0:?}");
        assert!(row1.contains("Game B") && row1.contains("200h"), "{row1:?}");
        assert!(!row0.contains("4830"), "raw value leaked: {row0:?}");
        assert!(!row1.contains("12000"), "raw value leaked: {row1:?}");
    }

    #[test]
    fn missing_value_label_falls_back_to_numeric_value() {
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Bars(BarsData {
                bars: vec![Bar {
                    label: "alpha".into(),
                    value: 42,
                    value_label: None,
                }],
            }),
        };
        let spec = RenderSpec::Short("list_ranking".into());
        let buf = render_with(&spec, &payload, 30, 1);
        let row0 = line_text(&buf, 0);
        assert!(row0.contains("42"), "{row0:?}");
    }

    #[test]
    fn style_none_omits_prefix() {
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "list_ranking", style = "none" }"#).unwrap();
        let p = payload(&[("alpha", 3), ("beta", 9)]);
        let buf = render_with(&w.render, &p, 30, 2);
        let r0 = line_text(&buf, 0);
        assert!(!r0.contains("1."), "row0: {r0:?}");
        // First in the fetcher's order (alpha) leads the rendered list now that auto-sort is gone.
        assert!(r0.trim_start().starts_with("alpha"), "row0: {r0:?}");
    }

    #[test]
    fn ties_preserve_input_order() {
        // Stable sort: same value → original order kept (alpha first since it's first in input).
        let p = payload(&[("alpha", 5), ("beta", 5), ("gamma", 5)]);
        let spec = RenderSpec::Short("list_ranking".into());
        let buf = render_with(&spec, &p, 30, 3);
        assert!(line_text(&buf, 0).contains("alpha"));
        assert!(line_text(&buf, 1).contains("beta"));
        assert!(line_text(&buf, 2).contains("gamma"));
    }

    #[test]
    fn align_center_offsets_block() {
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "list_ranking", align = "center" }"#).unwrap();
        let p = payload(&[("a", 1)]);
        let buf = render_with(&w.render, &p, 20, 1);
        let row = line_text(&buf, 0);
        // Single short row centered in width 20 should have leading whitespace.
        assert!(row.starts_with(' '), "expected leading spaces: {row:?}");
    }

    #[test]
    fn value_column_right_aligns_across_rows() {
        // Values 142 vs 9: the trailing column should line up so both end at the same column.
        let p = payload(&[("a", 142), ("b", 9)]);
        let spec = RenderSpec::Short("list_ranking".into());
        let buf = render_with(&spec, &p, 30, 2);
        let r0 = line_text(&buf, 0).trim_end().to_string();
        let r1 = line_text(&buf, 1).trim_end().to_string();
        assert!(r0.ends_with("142"), "row0 should end with 142: {r0:?}");
        assert!(r1.ends_with('9'), "row1 should end with 9: {r1:?}");
        assert_eq!(
            r0.len(),
            r1.len(),
            "trailing column should align: {r0:?} vs {r1:?}"
        );
    }

    #[test]
    fn empty_area_does_not_panic() {
        let p = payload(&[("a", 1)]);
        let spec = RenderSpec::Short("list_ranking".into());
        let _ = render_with(&spec, &p, 0, 0);
    }

    #[test]
    fn rank_prefix_falls_back_to_number_past_third_medal() {
        assert_eq!(rank_prefix(0, "medal"), "🥇");
        assert_eq!(rank_prefix(2, "medal"), "🥉");
        assert_eq!(rank_prefix(3, "medal"), "4.");
        assert_eq!(rank_prefix(0, "number"), "1.");
        assert_eq!(rank_prefix(0, "none"), "");
    }

    #[test]
    fn display_width_counts_medals_as_two_cells() {
        assert_eq!(display_width("🥇"), 2);
        assert_eq!(display_width("10."), 3);
    }
}
