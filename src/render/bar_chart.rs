use ratatui::{Frame, layout::Rect, widgets::BarChart};

use crate::payload::BarChartData;

pub fn render(frame: &mut Frame, area: Rect, data: &BarChartData) {
    let bars: Vec<(&str, u64)> = data
        .bars
        .iter()
        .map(|b| (b.label.as_str(), b.value))
        .collect();
    frame.render_widget(BarChart::default().data(&bars).bar_width(3), area);
}

#[cfg(test)]
mod tests {
    use crate::payload::{Bar, BarChartData, Body, Payload};
    use crate::render::test_utils::render_to_buffer;

    fn payload(bars: Vec<Bar>) -> Payload {
        Payload {
            title: None,
            icon: None,
            status: None,
            format: None,
            body: Body::BarChart(BarChartData { bars }),
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
