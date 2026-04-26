use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    buffer::Buffer,
    layout::{Alignment, Constraint, Flex, Layout as RatLayout, Position, Rect},
    style::{Modifier, Style},
    widgets::{
        Widget,
        calendar::{CalendarEventStore, Monthly},
    },
};
use time::{Date, Month};

use crate::options::OptionSchema;
use crate::payload::{Body, CalendarData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

/// Reserved forward-compat fields. Currently unused by the Monthly widget but accepted in
/// config so users can stage option values ahead of feature work.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Options {
    #[serde(default)]
    pub week_start: Option<String>,
    #[serde(default)]
    pub marker: Option<String>,
}

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
            // Parse extras so unknown keys still fail per `deny_unknown_fields`; values are
            // ignored until the underlying widget supports them.
            let _: Options = opts.parse_specific();
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
    let widget = Monthly::new(anchor, events)
        .default_style(Style::default().fg(theme.text))
        .show_month_header(panel_title)
        .show_weekdays_header(dim);
    // ratatui's `Monthly` compares an absolute `y` against `buf.area.height` when deciding
    // whether to draw each week row, so any non-zero viewport origin (subsequent inline
    // renders after the cursor has scrolled down) skips the date grid entirely. Render into
    // an origin-anchored off-screen buffer first, then blit into the frame.
    let off = render_offscreen(target.width, target.height, widget);
    blit(&off, frame.buffer_mut(), target);
}

fn render_offscreen<W: Widget>(width: u16, height: u16, widget: W) -> Buffer {
    let mut term = Terminal::new(TestBackend::new(width.max(1), height.max(1)))
        .expect("TestBackend::new is infallible");
    term.draw(|f| widget.render(f.area(), f.buffer_mut()))
        .expect("TestBackend draw is infallible");
    term.backend().buffer().clone()
}

fn blit(src: &Buffer, dst: &mut Buffer, target: Rect) {
    let dst_area = dst.area;
    for sy in 0..src.area.height {
        let dy = target.y + sy;
        if dy >= dst_area.bottom() {
            break;
        }
        for sx in 0..src.area.width {
            let dx = target.x + sx;
            if dx >= dst_area.right() {
                break;
            }
            if let (Some(src_cell), Some(dst_cell)) = (
                src.cell(Position::new(src.area.x + sx, src.area.y + sy)),
                dst.cell_mut(Position::new(dx, dy)),
            ) {
                *dst_cell = src_cell.clone();
            }
        }
    }
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
    fn renders_at_non_zero_viewport_y() {
        // Regression: ratatui's Monthly compares an absolute `y` against `buf.area.height`
        // when deciding whether to draw each week row. A non-zero viewport origin (subsequent
        // inline renders after a previous splash + prompt scrolls the cursor down) makes
        // every week row fail the guard, so the date grid silently disappears while the
        // month/weekday headers still render. Reproducing the failing condition requires a
        // buffer whose `area.y` is non-zero AND larger than `area.height`.
        use ratatui::{Terminal, TerminalOptions, Viewport, backend::TestBackend};
        let backend = TestBackend::new(24, 30);
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Inline(8),
            },
        )
        .unwrap();
        // Burn down the inline viewport to a high y so the buggy guard fires.
        for _ in 0..3 {
            terminal
                .draw(|f| f.render_widget(ratatui::widgets::Clear, f.area()))
                .unwrap();
            terminal.insert_before(8, |_| {}).unwrap();
        }
        let theme = Theme::default();
        let p = payload(2026, 4, Some(25));
        let Body::Calendar(data) = &p.body else {
            unreachable!()
        };
        terminal
            .draw(|f| {
                let area = f.area();
                render_calendar(f, area, data, &RenderOptions::default(), &theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut dump = String::new();
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
                .collect();
            dump.push_str(&row);
            dump.push('\n');
        }
        assert!(dump.contains("April"), "expected month header:\n{dump}");
        assert!(
            dump.contains("25"),
            "expected day 25 in calendar at non-zero viewport y:\n{dump}"
        );
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
