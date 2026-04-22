use ratatui::{Frame, layout::Rect};
use tui_piechart::{PieChart, PieSlice};

use crate::payload::{BarsData, Body};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::PALETTE_SERIES];

/// Pie-chart renderer. Consumes the same `Bars` shape as `chart_bar`, so one fetcher feeds
/// either — `render = "chart_pie"` vs `render = "chart_bar"` is a config choice. Slice colours
/// cycle through the shared `series` palette in input order; labels and percentages come from
/// the widget.
pub struct ChartPieRenderer;

impl Renderer for ChartPieRenderer {
    fn name(&self) -> &str {
        "chart_pie"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Bars]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
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
        if let Body::Bars(d) = body {
            render_pie(frame, area, d, theme);
        }
    }
}

fn render_pie(frame: &mut Frame, area: Rect, data: &BarsData, theme: &Theme) {
    let slices: Vec<PieSlice> = data
        .bars
        .iter()
        .enumerate()
        .map(|(i, b)| PieSlice::new(b.label.as_str(), b.value as f64, theme.series_color(i)))
        .collect();
    frame.render_widget(
        PieChart::new(slices)
            .show_legend(true)
            .show_percentages(true),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Bar, BarsData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(bars: Vec<Bar>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Bars(BarsData { bars }),
        }
    }

    fn render(bars: Vec<Bar>, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("chart_pie".into());
        render_to_buffer_with_spec(&payload(bars), Some(&spec), &registry, w, h)
    }

    fn bar(label: &str, value: u64) -> Bar {
        Bar {
            label: label.into(),
            value,
        }
    }

    #[test]
    fn renders_without_panicking() {
        let _ = render(
            vec![bar("rust", 45), bar("go", 30), bar("python", 25)],
            40,
            15,
        );
    }

    #[test]
    fn empty_bars_short_circuit_to_placeholder() {
        use crate::render::test_utils::line_text;
        let buf = render(vec![], 40, 5);
        let joined: String = (0..5).map(|y| line_text(&buf, y)).collect();
        assert!(
            joined.contains("nothing here yet"),
            "missing empty-state caption: {joined:?}"
        );
    }

    #[test]
    fn accepts_only_bars_shape() {
        let r = ChartPieRenderer;
        assert_eq!(r.accepts(), &[Shape::Bars]);
    }

    #[test]
    fn registered_under_chart_pie_name() {
        let r = Registry::with_builtins();
        assert!(r.get("chart_pie").is_some(), "chart_pie not registered");
    }

    #[test]
    fn legend_labels_appear_in_buffer() {
        use crate::render::test_utils::line_text;
        let buf = render(vec![bar("alpha", 10), bar("beta", 20)], 60, 20);
        let joined: String = (0..20).map(|y| line_text(&buf, y)).collect();
        assert!(joined.contains("alpha"), "alpha label missing: {joined:?}");
        assert!(joined.contains("beta"), "beta label missing: {joined:?}");
    }

    #[test]
    fn more_bars_than_palette_does_not_panic() {
        let palette_len = Theme::default().palette_series.len();
        let many: Vec<Bar> = (0..palette_len + 3)
            .map(|i| bar(&format!("b{i}"), (i as u64) + 1))
            .collect();
        let _ = render(many, 80, 30);
    }
}
