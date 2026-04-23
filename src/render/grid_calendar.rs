use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout as RatLayout, Rect},
    style::{Modifier, Style},
    widgets::calendar::{CalendarEventStore, Monthly},
};
use time::{Date, Month};

use crate::options::OptionSchema;
use crate::payload::{Body, CalendarData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::ACCENT_TODAY, theme::ACCENT_EVENT, theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "week_start",
        type_hint: "\"sun\" | \"mon\"",
        required: false,
        default: Some("\"sun\""),
        description: "Day the week starts on. Reserved for the future — ratatui's Monthly widget always renders Sun-first; the option is accepted so configs stay forward-compatible.",
    },
    OptionSchema {
        name: "marker",
        type_hint: "string",
        required: false,
        default: None,
        description: "Replacement glyph for event days. Reserved — the current Monthly widget tints event cells via style only; the option is accepted for forward compatibility.",
    },
];

/// Month-view calendar for the `Calendar` shape. Highlights `day` (today / focus) and marks
/// each day in `events`. Silently no-ops on invalid dates — a splash must never panic on bad
/// data flowing in from a plugin.
pub struct GridCalendarRenderer;

impl Renderer for GridCalendarRenderer {
    fn name(&self) -> &str {
        "grid_calendar"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Calendar]
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
        if let Body::Calendar(d) = body {
            render_calendar(frame, area, d, opts, theme);
        }
    }
}

/// `Monthly` renders 7 weekday columns × 3 cells each = 21 cells wide. Pinning the
/// natural width here lets us centre the grid inside a wider cell; without this, the
/// weekday header and dates glue to the left edge while the month-name header centres
/// against the full area, which reads as ragged under side-by-side layouts.
const MONTHLY_GRID_WIDTH: u16 = 21;

fn render_calendar(
    frame: &mut Frame,
    area: Rect,
    data: &CalendarData,
    opts: &RenderOptions,
    theme: &Theme,
) {
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
                .fg(theme.accent_today)
                .add_modifier(Modifier::BOLD),
        );
    }
    for day in &data.events {
        if let Ok(date) = Date::from_calendar_date(data.year, month, *day) {
            events.add(date, Style::default().fg(theme.accent_event));
        }
    }
    let panel_title = Style::default()
        .fg(theme.panel_title)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme.text_dim);
    let target = aligned_area(area, opts.align.as_deref());
    // `default_style` paints the grid of non-event day numbers + header so unmarked days
    // match the Splash text colour instead of leaking the terminal fg against the navy bg.
    // Show the month name and weekday labels so the grid is readable as a standalone block
    // instead of just bare numbers.
    frame.render_widget(
        Monthly::new(anchor, events)
            .default_style(Style::default().fg(theme.text))
            .show_month_header(panel_title)
            .show_weekdays_header(dim),
        target,
    );
}

fn aligned_area(area: Rect, align: Option<&str>) -> Rect {
    if area.width <= MONTHLY_GRID_WIDTH {
        return area;
    }
    let flex = match to_alignment(align) {
        Alignment::Center => Flex::Center,
        Alignment::Right => Flex::End,
        _ => Flex::Start,
    };
    let [slot] = RatLayout::horizontal([Constraint::Length(MONTHLY_GRID_WIDTH)])
        .flex(flex)
        .areas(area);
    slot
}

fn to_alignment(align: Option<&str>) -> Alignment {
    match align {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
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
        let spec = RenderSpec::Short("grid_calendar".into());
        let _ =
            render_to_buffer_with_spec(&payload(2026, 4, Some(21)), Some(&spec), &registry, 24, 9);
    }

    #[test]
    fn invalid_month_does_not_panic() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("grid_calendar".into());
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
        let spec = RenderSpec::Short("grid_calendar".into());
        let _ = render_to_buffer_with_spec(&p, Some(&spec), &registry, 24, 9);
    }
}
