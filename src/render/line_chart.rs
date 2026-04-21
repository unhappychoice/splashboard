use ratatui::{
    Frame,
    layout::Rect,
    symbols::Marker,
    widgets::{Axis, Chart, Dataset, GraphType},
};

use crate::payload::{LineChartData, LineSeries};

pub fn render(frame: &mut Frame, area: Rect, data: &LineChartData) {
    let datasets: Vec<Dataset> = data.series.iter().map(to_dataset).collect();
    let (x_bounds, y_bounds) = bounds(&data.series);
    let chart = Chart::new(datasets)
        .x_axis(Axis::default().bounds(x_bounds))
        .y_axis(Axis::default().bounds(y_bounds));
    frame.render_widget(chart, area);
}

fn to_dataset(series: &LineSeries) -> Dataset<'_> {
    Dataset::default()
        .name(series.name.clone())
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .data(&series.points)
}

fn bounds(series: &[LineSeries]) -> ([f64; 2], [f64; 2]) {
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
    use crate::payload::{Body, LineChartData, LineSeries, Payload};
    use crate::render::test_utils::render_to_buffer;

    fn payload(series: Vec<LineSeries>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::LineChart(LineChartData { series }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let s = LineSeries {
            name: "temp".into(),
            points: vec![(0.0, 20.0), (1.0, 21.5), (2.0, 19.8)],
        };
        let _ = render_to_buffer(&payload(vec![s]), 30, 10);
    }

    #[test]
    fn handles_empty_series() {
        let _ = render_to_buffer(&payload(vec![]), 30, 10);
    }
}
