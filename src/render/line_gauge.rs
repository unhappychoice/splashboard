use ratatui::{Frame, layout::Rect, widgets::LineGauge};

use crate::payload::{Body, RatioData};

use super::{RenderOptions, Renderer, Shape};

/// Compact progress indicator: a single-line bar, ratio filled. Good for dense dashboards where
/// the full-height `gauge` block renderer is too tall. Alternate renderer for the `Ratio` shape.
pub struct LineGaugeRenderer;

impl Renderer for LineGaugeRenderer {
    fn name(&self) -> &str {
        "line_gauge"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Ratio]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Ratio(d) = body {
            render_line_gauge(frame, area, d);
        }
    }
}

fn render_line_gauge(frame: &mut Frame, area: Rect, data: &RatioData) {
    let ratio = data.value.clamp(0.0, 1.0);
    let mut gauge = LineGauge::default().ratio(ratio);
    if let Some(label) = &data.label {
        gauge = gauge.label(label.clone());
    }
    frame.render_widget(gauge, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, RatioData};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(value: f64) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Ratio(RatioData {
                value,
                label: Some("%".into()),
            }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("line_gauge".into());
        let _ = render_to_buffer_with_spec(&payload(0.4), Some(&spec), &registry, 20, 3);
    }

    #[test]
    fn clamps_out_of_range() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("line_gauge".into());
        let _ = render_to_buffer_with_spec(&payload(1.7), Some(&spec), &registry, 20, 3);
        let _ = render_to_buffer_with_spec(&payload(-0.2), Some(&spec), &registry, 20, 3);
    }
}
