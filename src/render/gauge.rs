use ratatui::{Frame, layout::Rect, widgets::Gauge};

use crate::payload::GaugeData;

pub fn render(frame: &mut Frame, area: Rect, data: &GaugeData) {
    let ratio = data.value.clamp(0.0, 1.0);
    let mut gauge = Gauge::default().ratio(ratio);
    if let Some(label) = &data.label {
        gauge = gauge.label(label.clone());
    }
    frame.render_widget(gauge, area);
}

#[cfg(test)]
mod tests {
    use crate::payload::{Body, GaugeData, Payload};
    use crate::render::test_utils::render_to_buffer;

    fn payload(value: f64, label: Option<&str>) -> Payload {
        Payload {
            title: None,
            icon: None,
            status: None,
            format: None,
            body: Body::Gauge(GaugeData {
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
