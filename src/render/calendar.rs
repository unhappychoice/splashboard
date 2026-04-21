use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::calendar::{CalendarEventStore, Monthly},
};
use time::{Date, Month};

use crate::payload::{Body, CalendarData};

use super::{RenderOptions, Renderer, Shape};

/// Month-view calendar for the `Calendar` shape. Highlights `day` (today / focus) and marks
/// each day in `events`. Silently no-ops on invalid dates — a splash must never panic on bad
/// data flowing in from a plugin.
pub struct CalendarRenderer;

impl Renderer for CalendarRenderer {
    fn name(&self) -> &str {
        "calendar"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Calendar]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Calendar(d) = body {
            render_calendar(frame, area, d);
        }
    }
}

fn render_calendar(frame: &mut Frame, area: Rect, data: &CalendarData) {
    let Some(month) = month_from_u8(data.month) else {
        return;
    };
    let Ok(anchor) = Date::from_calendar_date(data.year, month, data.day.unwrap_or(1).max(1))
    else {
        return;
    };
    let mut events = CalendarEventStore::default();
    if let Some(d) = data.day
        && let Ok(today) = Date::from_calendar_date(data.year, month, d)
    {
        events.add(
            today,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    for day in &data.events {
        if let Ok(date) = Date::from_calendar_date(data.year, month, *day) {
            events.add(date, Style::default().fg(Color::Cyan));
        }
    }
    frame.render_widget(Monthly::new(anchor, events), area);
}

fn month_from_u8(m: u8) -> Option<Month> {
    match m {
        1 => Some(Month::January),
        2 => Some(Month::February),
        3 => Some(Month::March),
        4 => Some(Month::April),
        5 => Some(Month::May),
        6 => Some(Month::June),
        7 => Some(Month::July),
        8 => Some(Month::August),
        9 => Some(Month::September),
        10 => Some(Month::October),
        11 => Some(Month::November),
        12 => Some(Month::December),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{CalendarData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(year: i32, month: u8, day: Option<u8>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Calendar(CalendarData {
                year,
                month,
                day,
                events: Vec::new(),
            }),
        }
    }

    #[test]
    fn renders_a_month() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("calendar".into());
        let _ =
            render_to_buffer_with_spec(&payload(2026, 4, Some(21)), Some(&spec), &registry, 24, 9);
    }

    #[test]
    fn invalid_month_does_not_panic() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("calendar".into());
        let _ = render_to_buffer_with_spec(&payload(2026, 13, None), Some(&spec), &registry, 24, 9);
    }

    #[test]
    fn events_highlight_days() {
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Calendar(CalendarData {
                year: 2026,
                month: 4,
                day: Some(21),
                events: vec![5, 12, 28],
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("calendar".into());
        let _ = render_to_buffer_with_spec(&p, Some(&spec), &registry, 24, 9);
    }
}
