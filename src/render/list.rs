use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Style},
    widgets::{Row, Table},
};

use crate::payload::{Body, EntriesData, Entry, Status};

use super::{RenderOptions, Renderer, Shape};

pub struct ListRenderer;

impl Renderer for ListRenderer {
    fn name(&self) -> &str {
        "list"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Entries]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Entries(d) = body {
            render_entries(frame, area, d);
        }
    }
}

fn render_entries(frame: &mut Frame, area: Rect, data: &EntriesData) {
    let rows = data.items.iter().map(to_row);
    let widths = [Constraint::Percentage(40), Constraint::Percentage(60)];
    frame.render_widget(Table::new(rows, widths), area);
}

fn to_row(item: &Entry) -> Row<'_> {
    let mut row = Row::new(vec![
        item.key.clone(),
        item.value.clone().unwrap_or_default(),
    ]);
    if let Some(status) = item.status {
        row = row.style(Style::default().fg(status_color(status)));
    }
    row
}

fn status_color(status: Status) -> Color {
    match status {
        Status::Ok => Color::Green,
        Status::Warn => Color::Yellow,
        Status::Error => Color::Red,
    }
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
