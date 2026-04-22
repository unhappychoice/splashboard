use ratatui::{Frame, layout::Rect, style::Style, widgets::BarChart};

use crate::payload::{BarsData, Body};
use crate::theme::{self, ColorKey, Theme};

use super::{RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

pub struct ChartBarRenderer;

impl Renderer for ChartBarRenderer {
    fn name(&self) -> &str {
        "chart_bar"
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
    ) {
        if let Body::Bars(d) = body {
            render_bars(frame, area, d, theme);
        }
    }
}

fn render_bars(frame: &mut Frame, area: Rect, data: &BarsData, theme: &Theme) {
    let bars: Vec<(&str, u64)> = data
        .bars
        .iter()
        .map(|b| (b.label.as_str(), b.value))
        .collect();
    // `label_style` colours the per-bar category name, `value_style` the value printed
    // on top of each bar. Leave the bar fill to ratatui's default for now.
    frame.render_widget(
        BarChart::default()
            .data(&bars)
            .bar_width(3)
            .label_style(Style::default().fg(theme.text))
            .value_style(Style::default().fg(theme.text)),
        area,
    );
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
