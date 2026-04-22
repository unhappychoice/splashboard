use ratatui::{
    Frame,
    layout::Rect,
    style::Color,
    widgets::canvas::{Canvas, Points},
};

use crate::payload::{Body, PointSeries, PointSeriesData};

use super::{RenderOptions, Renderer, Shape};

/// Canvas-drawn scatter plot: dots only, no connecting lines. Alternate renderer for
/// `PointSeries`, paired with the Braille line renderer (`chart_line`) so users get to pick
/// "show me the trend" vs "show me each observation".
pub struct ChartScatterRenderer;

impl Renderer for ChartScatterRenderer {
    fn name(&self) -> &str {
        "chart_scatter"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::PointSeries]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::PointSeries(d) = body {
            render_scatter(frame, area, d);
        }
    }
}

fn render_scatter(frame: &mut Frame, area: Rect, data: &PointSeriesData) {
    let (x_bounds, y_bounds) = bounds(&data.series);
    let canvas = Canvas::default()
        .x_bounds(x_bounds)
        .y_bounds(y_bounds)
        .paint(|ctx| {
            for series in &data.series {
                ctx.draw(&Points {
                    coords: &series.points,
                    color: Color::Cyan,
                });
            }
        });
    frame.render_widget(canvas, area);
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
