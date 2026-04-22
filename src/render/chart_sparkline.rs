use ratatui::{Frame, layout::Rect, widgets::Sparkline};

use crate::payload::{Body, NumberSeriesData};

use super::{RenderOptions, Renderer, Shape};

pub struct ChartSparklineRenderer;

impl Renderer for ChartSparklineRenderer {
    fn name(&self) -> &str {
        "chart_sparkline"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::NumberSeries]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::NumberSeries(d) = body {
            render_sparkline(frame, area, d);
        }
    }
}

fn render_sparkline(frame: &mut Frame, area: Rect, data: &NumberSeriesData) {
    frame.render_widget(Sparkline::default().data(&data.values), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{NumberSeriesData, Payload};
    use crate::render::test_utils::render_to_buffer;

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
}
