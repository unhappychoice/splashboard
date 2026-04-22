use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    widgets::{Row, Table},
};

use crate::payload::{Body, EntriesData, Entry};
use crate::theme::{self, ColorKey, Theme};

use super::{RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::STATUS_OK, theme::STATUS_WARN, theme::STATUS_ERROR];

/// Key/value table renderer over the `Entries` shape. Internally a ratatui `Table` with two
/// columns (key, value) — the name reflects the widget, not the data, so future alternative
/// renderers for the same shape (card layout, inline chips) don't have to fight over "list".
pub struct GridTableRenderer;

impl Renderer for GridTableRenderer {
    fn name(&self) -> &str {
        "grid_table"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Entries]
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
        if let Body::Entries(d) = body {
            render_entries(frame, area, d, theme);
        }
    }
}

fn render_entries(frame: &mut Frame, area: Rect, data: &EntriesData, theme: &Theme) {
    let rows = data.items.iter().map(|e| to_row(e, theme));
    let widths = [Constraint::Percentage(40), Constraint::Percentage(60)];
    frame.render_widget(Table::new(rows, widths), area);
}

fn to_row<'a>(item: &'a Entry, theme: &Theme) -> Row<'a> {
    let mut row = Row::new(vec![
        item.key.clone(),
        item.value.clone().unwrap_or_default(),
    ]);
    if let Some(status) = item.status {
        row = row.style(Style::default().fg(super::status_badge::status_color(status, theme)));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{EntriesData, Entry, Payload, Status};
    use crate::render::test_utils::{line_text, render_to_buffer};

    fn payload(items: Vec<Entry>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Entries(EntriesData { items }),
        }
    }

    #[test]
    fn renders_key_and_value() {
        let p = payload(vec![Entry {
            key: "uptime".into(),
            value: Some("3d".into()),
            status: Some(Status::Ok),
        }]);
        let buf = render_to_buffer(&p, 30, 5);
        let row = line_text(&buf, 0);
        assert!(row.contains("uptime"));
        assert!(row.contains("3d"));
    }
}
