use ratatui::{Frame, layout::Rect, widgets::Gauge};

use crate::payload::{Body, RatioData};
use crate::theme::Theme;

use super::{RenderOptions, Renderer, Shape};

pub struct GaugeCircleRenderer;

impl Renderer for GaugeCircleRenderer {
    fn name(&self) -> &str {
        "gauge_circle"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Ratio]
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        _opts: &RenderOptions,
        _theme: &Theme,
    ) {
        if let Body::Ratio(d) = body {
            render_gauge(frame, area, d);
        }
    }
}

fn render_gauge(frame: &mut Frame, area: Rect, data: &RatioData) {
    let ratio = data.value.clamp(0.0, 1.0);
    let mut gauge = Gauge::default().ratio(ratio);
    if let Some(label) = &data.label {
        gauge = gauge.label(label.clone());
    }
    frame.render_widget(gauge, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, RatioData};
    use crate::render::test_utils::render_to_buffer;

    fn payload(value: f64, label: Option<&str>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Ratio(RatioData {
                value,
                label: label.map(String::from),
            }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let _ = render_to_buffer(&payload(0.5, Some("CPU")), 20, 5);
    }

    #[test]
    fn clamps_out_of_range_value() {
        let _ = render_to_buffer(&payload(1.7, None), 20, 5);
        let _ = render_to_buffer(&payload(-0.2, None), 20, 5);
    }
}
