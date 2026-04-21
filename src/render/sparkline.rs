use ratatui::{Frame, layout::Rect, widgets::Sparkline};

use crate::payload::SparklineData;

pub fn render(frame: &mut Frame, area: Rect, data: &SparklineData) {
    frame.render_widget(Sparkline::default().data(&data.values), area);
}

#[cfg(test)]
mod tests {
    use crate::payload::{Body, Payload, SparklineData};
    use crate::render::test_utils::render_to_buffer;

    fn payload(values: Vec<u64>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Sparkline(SparklineData { values }),
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
