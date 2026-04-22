use ratatui::{Frame, layout::Rect, widgets::Sparkline};

use crate::payload::{Body, NumberSeriesData};
use crate::theme::Theme;

use super::{Registry, RenderOptions, Renderer, Shape};

pub struct ChartSparklineRenderer;

impl Renderer for ChartSparklineRenderer {
    fn name(&self) -> &str {
        "chart_sparkline"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::NumberSeries]
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        _opts: &RenderOptions,
        _theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::NumberSeries(d) = body {
            render_sparkline(frame, area, d);
        }
    }
}

fn render_sparkline(frame: &mut Frame, area: Rect, data: &NumberSeriesData) {
    let start = data.values.len().saturating_sub(area.width as usize);
    let visible = &data.values[start..];
    frame.render_widget(Sparkline::default().data(visible), area);
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

    /// Regression for the ratatui `Sparkline` pitfall: its default `render` takes the *first*
    /// `min(area.width, data.len())` values, so feeding a long series (e.g. 364 daily commits)
    /// into a narrow area drops everything after the leading cells. For sparse "recent spike"
    /// data that's indistinguishable from an empty widget. We slice the tail before handing off
    /// so the most-recent cells are what get drawn.
    #[test]
    fn tail_window_is_rendered_when_data_exceeds_width() {
        let mut values = vec![0u64; 100];
        values[97] = 50;
        values[98] = 70;
        values[99] = 10;
        let buf = render_to_buffer(&payload(values), 30, 3);
        let rightmost: String = (buf.area.width - 3..buf.area.width)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(
            rightmost.chars().any(|c| c != ' '),
            "expected recent spikes in rightmost cells, got: {rightmost:?}"
        );
    }
}
