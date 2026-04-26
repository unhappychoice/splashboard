use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    symbols::Marker,
    widgets::{
        Block, Borders,
        canvas::{Canvas, Points},
    },
};

use crate::options::OptionSchema;
use crate::payload::{Body, PointSeries, PointSeriesData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub marker: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::PALETTE_SERIES, theme::PANEL_BORDER];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "axes",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Draw a bordered block around the canvas so axes read as framed.",
    },
    OptionSchema {
        name: "marker",
        type_hint: "\"dot\" | \"block\" | \"bar\" | \"braille\" | \"half_block\"",
        required: false,
        default: Some("\"dot\""),
        description: "Scatter glyph family, mapped to ratatui's `symbols::Marker` variants. Unknown names fall back to `dot`.",
    },
];

/// Canvas-drawn scatter plot: dots only, no connecting lines. Alternate renderer for
/// `PointSeries`, paired with the Braille line renderer (`chart_line`) so users get to pick
/// "show me the trend" vs "show me each observation".
pub struct ChartScatterRenderer;

impl Renderer for ChartScatterRenderer {
    fn name(&self) -> &str {
        "chart_scatter"
    }
    fn description(&self) -> &'static str {
        "Canvas scatter plot of `PointSeries` — dots only, no connecting line, with per-series palette colours and a configurable marker glyph. Pick this over `chart_line` when each observation matters more than the trend between them."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::PointSeries]
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
        if let Body::PointSeries(d) = body {
            render_scatter(frame, area, d, opts, theme);
        }
    }
}

fn render_scatter(
    frame: &mut Frame,
    area: Rect,
    data: &PointSeriesData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let specific: Options = opts.parse_specific();
    let (x_bounds, y_bounds) = bounds(&data.series);
    let mut canvas = Canvas::default()
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .marker(parse_marker(specific.marker.as_deref()))
        .paint(|ctx| {
            for (i, series) in data.series.iter().enumerate() {
                ctx.draw(&Points {
                    coords: &series.points,
                    color: theme.series_color(i),
                });
            }
        });
    if opts.axes.unwrap_or(false) {
        canvas = canvas.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.panel_border)),
        );
    }
    frame.render_widget(canvas, area);
}

fn parse_marker(marker: Option<&str>) -> Marker {
    match marker {
        Some("block") => Marker::Block,
        Some("bar") => Marker::Bar,
        Some("braille") => Marker::Braille,
        Some("half_block") => Marker::HalfBlock,
        _ => Marker::Dot,
    }
}

fn bounds(series: &[PointSeries]) -> ([f64; 2], [f64; 2]) {
    let points: Vec<(f64, f64)> = series
        .iter()
        .flat_map(|s| s.points.iter().copied())
        .collect();
    if points.is_empty() {
        return ([0.0, 1.0], [0.0, 1.0]);
    }
    let (xs, ys): (Vec<f64>, Vec<f64>) = points.into_iter().unzip();
    ([min(&xs), max(&xs)], [min(&ys), max(&ys)])
}

fn min(xs: &[f64]) -> f64 {
    xs.iter().copied().fold(f64::INFINITY, f64::min)
}

fn max(xs: &[f64]) -> f64 {
    xs.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, PointSeries, PointSeriesData};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(series: Vec<PointSeries>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::PointSeries(PointSeriesData { series }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let s = PointSeries {
            name: "temp".into(),
            points: vec![(0.0, 20.0), (1.0, 21.5), (2.0, 19.8)],
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("chart_scatter".into());
        let _ = render_to_buffer_with_spec(&payload(vec![s]), Some(&spec), &registry, 30, 10);
    }

    #[test]
    fn handles_empty_series() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("chart_scatter".into());
        let _ = render_to_buffer_with_spec(&payload(vec![]), Some(&spec), &registry, 30, 10);
    }
}
