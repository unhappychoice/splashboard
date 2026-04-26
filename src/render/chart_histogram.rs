use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Bar, BarChart, BarGroup, Paragraph},
};

use crate::options::OptionSchema;
use crate::payload::{Body, NumberSeriesData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub bins: Option<u16>,
    #[serde(default)]
    pub axis_labels: Option<bool>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::TEXT_DIM];

const MIN_AUTO_BINS: u16 = 5;

/// Minimum chart-area height that still leaves a usable row of bars (≥ 2) when both axis-label
/// rows are reserved. Below this we drop the labels rather than the bars.
const MIN_HEIGHT_FOR_AXIS_LABELS: u16 = 4;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "label",
        type_hint: "string",
        required: false,
        default: None,
        description: "Optional inline prefix rendered to the left of the histogram bars.",
    },
    OptionSchema {
        name: "bins",
        type_hint: "positive integer",
        required: false,
        default: Some("auto"),
        description: "Number of buckets the input is binned into. Auto picks `ceil(sqrt(n))` clamped to `[5, area width]`; explicit values clamp to `[1, area width]`.",
    },
    OptionSchema {
        name: "axis_labels",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Render the value range (min / max) along the bottom edge and the peak bucket count along the top edge. Auto-skipped when the chart area is shorter than 4 rows so bars keep usable space.",
    },
];

pub struct ChartHistogramRenderer;

impl Renderer for ChartHistogramRenderer {
    fn name(&self) -> &str {
        "chart_histogram"
    }
    fn description(&self) -> &'static str {
        "Vertical bar chart from a `NumberSeries`, binned into equal-width buckets. Optional `axis_labels` print the peak count along the top edge and the value range along the bottom; pairs with `chart_sparkline` when the same series should look bigger and shaped instead of compact."
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
            render_histogram(frame, area, d, opts, theme);
        }
    }
}

fn render_histogram(
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
    if chart_area.width == 0 || chart_area.height == 0 || data.values.is_empty() {
        return;
    }
    let specific: Options = opts.parse_specific();
    let want_axes = specific.axis_labels.unwrap_or(false);
    let (top_label_area, bars_area, bottom_label_area) = split_axis_areas(chart_area, want_axes);
    let bins = resolve_bins(specific.bins, data.values.len(), bars_area.width);
    let counts = boost_to_visible(bucket_counts(&data.values, bins), bars_area.height);
    let bars: Vec<Bar> = counts
        .iter()
        .map(|&c| Bar::default().value(c).text_value(String::new()))
        .collect();
    let bar_width = compute_bar_width(bars_area.width, bins);
    frame.render_widget(
        BarChart::default()
            .data(BarGroup::default().bars(&bars))
            .bar_width(bar_width)
            .bar_gap(0)
            .bar_style(Style::default().fg(theme.text)),
        bars_area,
    );
    if let (Some(top), Some(bottom)) = (top_label_area, bottom_label_area) {
        let peak = counts.iter().copied().max().unwrap_or(0);
        let (min, max) = (
            *data.values.iter().min().unwrap(),
            *data.values.iter().max().unwrap(),
        );
        let bars_span = (bar_width.saturating_mul(bins)).min(bars_area.width);
        let aligned_bottom = Rect {
            x: bottom.x,
            y: bottom.y,
            width: bars_span,
            height: bottom.height,
        };
        draw_axis_labels(frame, top, aligned_bottom, peak, min, max, theme);
    }
}

/// Splits the chart area into (optional top label row, bars row, optional bottom label row). Top
/// and bottom rows are reserved only when `want_axes` is true and the area can spare the height;
/// otherwise the bars expand to the full area.
fn split_axis_areas(area: Rect, want_axes: bool) -> (Option<Rect>, Rect, Option<Rect>) {
    if !want_axes || area.height < MIN_HEIGHT_FOR_AXIS_LABELS {
        return (None, area, None);
    }
    let [top, mid, bot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    (Some(top), mid, Some(bot))
}

fn draw_axis_labels(
    frame: &mut Frame,
    top: Rect,
    bottom: Rect,
    peak: u64,
    min: u64,
    max: u64,
    theme: &Theme,
) {
    let dim = Style::default().fg(theme.text_dim);
    frame.render_widget(Paragraph::new(format!("{peak}")).style(dim), top);
    let min_str = format!("{min}");
    let max_str = format!("{max}");
    let pad = bottom
        .width
        .saturating_sub((min_str.chars().count() + max_str.chars().count()) as u16);
    let bottom_text = format!("{min_str}{}{max_str}", " ".repeat(pad as usize));
    frame.render_widget(Paragraph::new(bottom_text).style(dim), bottom);
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

/// Stretches each bar to fill its share of the bars area. With `bins=10` in a 50-cell area
/// every bar gets 5 cells; previously they were locked at 1 cell wide and the histogram only
/// occupied the leftmost slice of its slot.
fn compute_bar_width(area_width: u16, bins: u16) -> u16 {
    let bins = bins.max(1);
    (area_width / bins).max(1)
}

fn resolve_bins(requested: Option<u16>, sample_count: usize, width: u16) -> u16 {
    let max_bins = width.max(1);
    if let Some(b) = requested {
        return b.max(1).min(max_bins);
    }
    let auto = (sample_count as f64).sqrt().ceil() as u16;
    auto.clamp(MIN_AUTO_BINS.min(max_bins), max_bins)
}

/// Lifts every non-zero count up to the smallest value that still renders at least one
/// sub-cell glyph. ratatui's `BarChart` computes `sub_cells = count * 8 * height / peak` and
/// floors the result, so any count below `peak / (8 * height)` rounds to 0 and disappears
/// from a skewed distribution's long tail. Keep "this bin has data" visible — the loss of
/// 1-vs-2 resolution at sub-cell heights is a small price to stop bins of count=1 from
/// silently vanishing under a `peak=51` mid-bin.
fn boost_to_visible(counts: Vec<u64>, area_height: u16) -> Vec<u64> {
    if area_height == 0 {
        return counts;
    }
    let peak = counts.iter().copied().max().unwrap_or(0);
    if peak == 0 {
        return counts;
    }
    let denom = 8u64 * area_height as u64;
    let min_visible = peak.div_ceil(denom);
    counts
        .into_iter()
        .map(|c| if c == 0 { 0 } else { c.max(min_visible) })
        .collect()
}

/// Distributes `values` into `bins` equal-width buckets between `min` and `max`. The top edge
/// (`v == max`) is folded into the last bucket so it doesn't escape the array on rounding.
fn bucket_counts(values: &[u64], bins: u16) -> Vec<u64> {
    let bins = bins.max(1) as usize;
    if values.is_empty() {
        return vec![0; bins];
    }
    let min = *values.iter().min().unwrap();
    let max = *values.iter().max().unwrap();
    if min == max {
        let mut out = vec![0u64; bins];
        out[0] = values.len() as u64;
        return out;
    }
    let range = (max - min) as f64;
    values.iter().fold(vec![0u64; bins], |mut acc, &v| {
        let pos = ((v - min) as f64 / range) * bins as f64;
        let idx = (pos as usize).min(bins - 1);
        acc[idx] += 1;
        acc
    })
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

    fn histogram_spec(toml_inline: &str) -> RenderSpec {
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(&format!("render = {toml_inline}")).unwrap();
        w.render
    }

    #[test]
    fn renders_without_panicking() {
        let _ = render_to_buffer(&payload(vec![1, 3, 2, 5, 4, 6, 7, 7, 8, 2]), 30, 6);
    }

    #[test]
    fn handles_empty_values() {
        let _ = render_to_buffer(&payload(vec![]), 30, 6);
    }

    #[test]
    fn narrow_area_does_not_panic() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("chart_histogram".into());
        let _ =
            render_to_buffer_with_spec(&payload(vec![1, 2, 3, 4, 5]), Some(&spec), &registry, 1, 1);
    }

    #[test]
    fn single_value_collapses_to_first_bucket() {
        let counts = bucket_counts(&[42, 42, 42, 42], 5);
        assert_eq!(counts, vec![4, 0, 0, 0, 0]);
    }

    #[test]
    fn even_distribution_spreads_across_bins() {
        let values: Vec<u64> = (0..20).collect();
        let counts = bucket_counts(&values, 4);
        assert_eq!(counts.iter().sum::<u64>(), 20);
        assert!(
            counts.iter().all(|&c| c > 0),
            "every bin should be hit: {counts:?}"
        );
    }

    #[test]
    fn extremes_land_in_first_and_last_bin() {
        let counts = bucket_counts(&[0, 100], 5);
        assert_eq!(counts[0], 1, "min should land in first bucket");
        assert_eq!(counts[4], 1, "max should land in last bucket");
    }

    #[test]
    fn empty_values_returns_zeroed_buckets() {
        let counts = bucket_counts(&[], 4);
        assert_eq!(counts, vec![0, 0, 0, 0]);
    }

    #[test]
    fn resolve_bins_clamps_requested() {
        assert_eq!(resolve_bins(Some(8), 100, 40), 8);
        assert_eq!(resolve_bins(Some(0), 10, 40), 1);
        assert_eq!(resolve_bins(Some(100), 10, 30), 30);
    }

    #[test]
    fn resolve_bins_auto_uses_sqrt() {
        assert_eq!(resolve_bins(None, 100, 40), 10);
    }

    #[test]
    fn resolve_bins_auto_floors_to_minimum() {
        assert_eq!(resolve_bins(None, 4, 40), MIN_AUTO_BINS);
    }

    #[test]
    fn resolve_bins_auto_respects_narrow_width() {
        assert_eq!(resolve_bins(None, 100, 3), 3);
    }

    #[test]
    fn label_option_prefixes_chart() {
        let registry = Registry::with_builtins();
        let spec = histogram_spec(r#"{ type = "chart_histogram", label = "lat" }"#);
        let buf = render_to_buffer_with_spec(
            &payload(vec![1, 2, 3, 4, 5, 6, 7, 8]),
            Some(&spec),
            &registry,
            30,
            5,
        );
        let row = line_text(&buf, 0);
        assert!(row.starts_with("lat"), "missing label prefix: {row:?}");
    }

    #[test]
    fn explicit_bins_option_parses() {
        let spec = histogram_spec(r#"{ type = "chart_histogram", bins = 12 }"#);
        let parsed: Options = spec.options().parse_specific();
        assert_eq!(parsed.bins, Some(12));
    }

    #[test]
    fn axis_labels_option_parses() {
        let spec = histogram_spec(r#"{ type = "chart_histogram", axis_labels = true }"#);
        let parsed: Options = spec.options().parse_specific();
        assert_eq!(parsed.axis_labels, Some(true));
    }

    #[test]
    fn axis_labels_renders_min_max_and_peak() {
        let registry = Registry::with_builtins();
        let spec = histogram_spec(r#"{ type = "chart_histogram", axis_labels = true, bins = 5 }"#);
        let buf = render_to_buffer_with_spec(
            &payload(vec![10, 20, 30, 40, 50, 50, 50, 80, 90, 100]),
            Some(&spec),
            &registry,
            30,
            6,
        );
        let top = line_text(&buf, 0);
        let bottom = line_text(&buf, 5);
        assert!(
            top.trim_end().starts_with('3'),
            "expected peak count on top: {top:?}"
        );
        assert!(
            bottom.trim_start().starts_with("10"),
            "expected min on bottom-left: {bottom:?}"
        );
        assert!(
            bottom.trim_end().ends_with("100"),
            "expected max on bottom-right: {bottom:?}"
        );
    }

    #[test]
    fn axis_labels_align_with_bar_span_not_chart_area() {
        // bins=4 in width=30 → bar_width=7, bars span 28 cells (cols 0..28). Max should land
        // ending at col 28, not at col 30 — otherwise the label points into empty padding.
        let registry = Registry::with_builtins();
        let spec = histogram_spec(r#"{ type = "chart_histogram", axis_labels = true, bins = 4 }"#);
        let buf = render_to_buffer_with_spec(
            &payload(vec![0, 10, 50, 100]),
            Some(&spec),
            &registry,
            30,
            6,
        );
        let bottom = line_text(&buf, 5);
        assert!(
            bottom.starts_with('0'),
            "min should sit at the leftmost bar: {bottom:?}"
        );
        assert_eq!(
            &bottom[25..28],
            "100",
            "max should hug the rightmost bar (col 28), not the area edge: {bottom:?}"
        );
        assert_eq!(
            bottom.trim_end_matches(' ').len(),
            28,
            "no labels should land in the padding past the bars: {bottom:?}"
        );
    }

    #[test]
    fn axis_labels_skipped_when_height_too_small() {
        let registry = Registry::with_builtins();
        let spec = histogram_spec(r#"{ type = "chart_histogram", axis_labels = true }"#);
        // height=3 < MIN_HEIGHT_FOR_AXIS_LABELS=4 → bars take the full area, no label rows.
        let buf = render_to_buffer_with_spec(
            &payload(vec![1, 2, 3, 4, 5, 6, 7, 8]),
            Some(&spec),
            &registry,
            20,
            3,
        );
        let bottom = line_text(&buf, 2);
        assert!(
            !bottom.trim_start().starts_with('1') || bottom.contains('█'),
            "labels should be suppressed and bars present: {bottom:?}"
        );
    }

    #[test]
    fn split_axis_areas_reserves_label_rows_when_tall_enough() {
        let area = Rect::new(0, 0, 30, 6);
        let (top, mid, bot) = split_axis_areas(area, true);
        assert!(top.is_some());
        assert!(bot.is_some());
        assert_eq!(mid.height, 4);
    }

    #[test]
    fn boost_to_visible_lifts_small_counts_above_subcell_threshold() {
        // peak=51, height=4 → denom=32, min_visible = ceil(51/32) = 2. Counts of 1 get
        // bumped to 2 so the long tail of a skewed distribution stays drawable.
        let counts = vec![7, 23, 51, 9, 4, 2, 1, 1, 1, 1];
        assert_eq!(
            boost_to_visible(counts, 4),
            vec![7, 23, 51, 9, 4, 2, 2, 2, 2, 2],
        );
    }

    #[test]
    fn boost_to_visible_preserves_zero_counts() {
        assert_eq!(
            boost_to_visible(vec![0, 1, 0, 5, 0], 4),
            vec![0, 1, 0, 5, 0],
        );
    }

    #[test]
    fn boost_to_visible_no_op_when_height_zero() {
        assert_eq!(boost_to_visible(vec![1, 5, 1], 0), vec![1, 5, 1]);
    }

    #[test]
    fn bar_width_fills_available_area() {
        assert_eq!(compute_bar_width(50, 10), 5);
        assert_eq!(compute_bar_width(58, 10), 5);
        assert_eq!(compute_bar_width(30, 5), 6);
        assert_eq!(compute_bar_width(20, 30), 1);
        assert_eq!(compute_bar_width(0, 5), 1);
        // bins == 0 coerces to 1, giving the single bar the full area.
        assert_eq!(compute_bar_width(10, 0), 10);
    }

    #[test]
    fn split_axis_areas_collapses_when_disabled() {
        let area = Rect::new(0, 0, 30, 10);
        let (top, mid, bot) = split_axis_areas(area, false);
        assert!(top.is_none());
        assert!(bot.is_none());
        assert_eq!(mid.height, 10);
    }
}
