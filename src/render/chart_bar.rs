use ratatui::{
    Frame,
    layout::{Direction, Rect},
    style::Style,
    widgets::BarChart,
};

use crate::options::OptionSchema;
use crate::payload::{BarsData, Body};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "horizontal",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Orient bars horizontally instead of vertical. Useful for wide/narrow widget slots.",
    },
    OptionSchema {
        name: "stacked",
        type_hint: "bool",
        required: false,
        default: Some("false"),
        description: "Placeholder — the current `Bars` shape is single-series, so this is a no-op until the shape gains series support. Accepted so configs stay forward-compatible.",
    },
    OptionSchema {
        name: "max_bars",
        type_hint: "positive integer",
        required: false,
        default: Some("all bars"),
        description: "Cap on rendered bars, keeping the top N by value. Omit to render every bar.",
    },
    OptionSchema {
        name: "bar_width",
        type_hint: "cells (u16)",
        required: false,
        default: Some("3 for vertical, 1 for horizontal"),
        description: "Thickness of each bar along the axis perpendicular to its growth. Horizontal bars default to 1 cell so several fit into a compact slot without stacking tall; vertical bars default to 3 so labels under them have room.",
    },
];

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
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::Bars(d) = body {
            render_bars(frame, area, d, opts, theme);
        }
    }
}

fn render_bars(
    frame: &mut Frame,
    area: Rect,
    data: &BarsData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let capped = capped_bars(data, opts.max_bars);
    let bars: Vec<(&str, u64)> = capped.iter().map(|b| (b.0.as_str(), b.1)).collect();
    let horizontal = opts.horizontal.unwrap_or(false);
    let direction = if horizontal {
        Direction::Horizontal
    } else {
        Direction::Vertical
    };
    let bar_width = opts.bar_width.unwrap_or(if horizontal { 1 } else { 3 });
    frame.render_widget(
        BarChart::default()
            .data(&bars)
            .bar_width(bar_width)
            .direction(direction)
            .label_style(Style::default().fg(theme.text))
            .value_style(Style::default().fg(theme.text)),
        area,
    );
}

/// Returns the bars the renderer should draw. `max_bars` keeps the top N by value; omitting
/// the cap leaves the payload order untouched so callers with curated sequencing (most
/// recent → oldest, alphabetical) still get what they fed in.
fn capped_bars(data: &BarsData, cap: Option<usize>) -> Vec<(String, u64)> {
    let mut bars: Vec<(String, u64)> = data
        .bars
        .iter()
        .map(|b| (b.label.clone(), b.value))
        .collect();
    if let Some(n) = cap
        && bars.len() > n
    {
        bars.sort_by_key(|b| std::cmp::Reverse(b.1));
        bars.truncate(n);
    }
    bars
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

    #[test]
    fn max_bars_keeps_top_values() {
        let bars = vec![
            Bar {
                label: "a".into(),
                value: 3,
            },
            Bar {
                label: "b".into(),
                value: 9,
            },
            Bar {
                label: "c".into(),
                value: 1,
            },
            Bar {
                label: "d".into(),
                value: 5,
            },
        ];
        let out = capped_bars(&BarsData { bars }, Some(2));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "b");
        assert_eq!(out[1].0, "d");
    }

    #[test]
    fn max_bars_absent_preserves_order() {
        let bars = vec![
            Bar {
                label: "a".into(),
                value: 1,
            },
            Bar {
                label: "b".into(),
                value: 9,
            },
        ];
        let out = capped_bars(&BarsData { bars }, None);
        assert_eq!(out[0].0, "a");
    }
}
