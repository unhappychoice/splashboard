use ratatui::{Frame, layout::Rect, widgets::BarChart};

use crate::payload::{BarsData, Body};

use super::{RenderOptions, Renderer, Shape};

pub struct ChartBarRenderer;

impl Renderer for ChartBarRenderer {
    fn name(&self) -> &str {
        "chart_bar"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Bars]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Bars(d) = body {
            render_bars(frame, area, d);
        }
    }
}

fn render_bars(frame: &mut Frame, area: Rect, data: &BarsData) {
    let bars: Vec<(&str, u64)> = data
        .bars
        .iter()
        .map(|b| (b.label.as_str(), b.value))
        .collect();
    frame.render_widget(BarChart::default().data(&bars).bar_width(3), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Bar, BarsData, Payload};
    use crate::render::test_utils::render_to_buffer;

    fn payload(bars: Vec<Bar>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Bars(BarsData { bars }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let bars = vec![
            Bar {
                label: "a".into(),
                value: 3,
            },
            Bar {
                label: "b".into(),
                value: 7,
            },
            Bar {
                label: "c".into(),
                value: 5,
            },
        ];
        let _ = render_to_buffer(&payload(bars), 30, 10);
    }

    #[test]
    fn handles_empty_bars() {
        let _ = render_to_buffer(&payload(vec![]), 30, 10);
    }
}
