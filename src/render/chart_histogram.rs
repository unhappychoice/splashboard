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

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const MIN_AUTO_BINS: u16 = 5;

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
];

pub struct ChartHistogramRenderer;

impl Renderer for ChartHistogramRenderer {
    fn name(&self) -> &str {
        "chart_histogram"
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
    let bins = resolve_bins(opts.bins, data.values.len(), chart_area.width);
    let counts = bucket_counts(&data.values, bins);
    let bars: Vec<Bar> = counts
        .iter()
        .map(|&c| Bar::default().value(c).text_value(String::new()))
        .collect();
    frame.render_widget(
        BarChart::default()
            .data(BarGroup::default().bars(&bars))
            .bar_width(1)
            .bar_gap(0)
            .bar_style(Style::default().fg(theme.text)),
        chart_area,
    );
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

fn resolve_bins(requested: Option<u16>, sample_count: usize, width: u16) -> u16 {
    let max_bins = width.max(1);
    if let Some(b) = requested {
        return b.max(1).min(max_bins);
    }
    let auto = (sample_count as f64).sqrt().ceil() as u16;
    auto.clamp(MIN_AUTO_BINS.min(max_bins), max_bins)
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
        assert_eq!(spec.options().bins, Some(12));
    }
}
