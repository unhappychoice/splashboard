use ratatui::{Frame, layout::Rect, style::Style, widgets::Sparkline};

use crate::payload::{Body, NumberSeriesData};
use crate::theme::Theme;

use super::{Registry, RenderOptions, Renderer, Shape};

/// Baseline glyph painted at each 0-value column. Deliberately distinct from ratatui's
/// smallest bar glyph (`▁`), which is what a tiny non-zero value would land on — we want
/// "no activity" to read differently from "1 out of max=74".
const ZERO_BASELINE: &str = "·";

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
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::NumberSeries(d) = body {
            render_sparkline(frame, area, d, theme);
        }
    }
}

fn render_sparkline(frame: &mut Frame, area: Rect, data: &NumberSeriesData, theme: &Theme) {
    let start = data.values.len().saturating_sub(area.width as usize);
    let visible = &data.values[start..];
    frame.render_widget(Sparkline::default().data(visible), area);
    draw_zero_baseline(frame, area, visible, theme);
}

/// ratatui `Sparkline` renders a 0-value column as empty space, which makes a sparse series
/// ("nothing happened for 50 days") visually indistinguishable from "no data / padding".
/// Paint [`ZERO_BASELINE`] on the bottom row at each 0-value column after the sparkline
/// draws, so every point in the visible window registers as a tick.
fn draw_zero_baseline(frame: &mut Frame, area: Rect, visible: &[u64], theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let bottom = area.y + area.height - 1;
    let style = Style::default().fg(theme.text_dim);
    for (i, &value) in visible.iter().enumerate() {
        if value != 0 {
            continue;
        }
        let x = area.x + i as u16;
        if x >= area.x + area.width {
            break;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((x, bottom)) {
            cell.set_symbol(ZERO_BASELINE);
            cell.set_style(style);
        }
    }
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

    /// ratatui renders value=0 as empty, so a sparse series ("no activity for a week") reads
    /// as "no data". We post-draw `·` on the bottom row at each zero-column so every visible
    /// datum leaves a mark — and a *different* mark than ratatui's smallest bar glyph (`▁`),
    /// which is what a value of "1 out of max=74" lands on.
    #[test]
    fn zero_values_draw_baseline_tick() {
        let values = vec![0u64, 5, 0, 0, 8, 0];
        let buf = render_to_buffer(&payload(values.clone()), 6, 3);
        let bottom_row: String = (0..6)
            .map(|x| buf.cell((x, 2)).unwrap().symbol().to_string())
            .collect();
        for (x, &v) in values.iter().enumerate() {
            if v == 0 {
                assert_eq!(
                    buf.cell((x as u16, 2)).unwrap().symbol(),
                    "·",
                    "zero column {x} should carry baseline, got row: {bottom_row:?}"
                );
            }
        }
    }
}
