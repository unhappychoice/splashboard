use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Style},
    widgets::{Row, Table},
};

use crate::payload::{ListData, ListItem, Status};

pub fn render(frame: &mut Frame, area: Rect, data: &ListData) {
    let rows = data.items.iter().map(to_row);
    let widths = [Constraint::Percentage(40), Constraint::Percentage(60)];
    frame.render_widget(Table::new(rows, widths), area);
}

fn to_row(item: &ListItem) -> Row<'_> {
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
    use crate::payload::{Body, ListData, ListItem, Payload, Status};
    use crate::render::test_utils::{line_text, render_to_buffer};

    fn payload(items: Vec<ListItem>) -> Payload {
        Payload {
            title: None,
            icon: None,
            status: None,
            format: None,
            body: Body::List(ListData { items }),
        }
    }

    #[test]
    fn renders_key_and_value() {
        let p = payload(vec![ListItem {
            key: "uptime".into(),
            value: Some("3d".into()),
            status: Some(Status::Ok),
        }]);
        let buf = render_to_buffer(&p, 30, 5);
        let row = line_text(&buf, 1);
        assert!(row.contains("uptime"));
        assert!(row.contains("3d"));
    }
}
