use chrono::{Datelike, TimeZone, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::payload::{Body, Status, TimelineData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT_DIM,
    theme::TEXT_SECONDARY,
    theme::TEXT,
];

/// Time-stamped event list. Each row shows a compact relative prefix (`"3h"`, `"2d"`, `"Apr 5"`)
/// computed at draw time against the current clock — cached payloads never freeze a stale
/// "ago" string. Optional `detail` hangs under the title; `status` colours the title.
pub struct ListTimelineRenderer;

const SEPARATOR: &str = " │ ";

impl Renderer for ListTimelineRenderer {
    fn name(&self) -> &str {
        "list_timeline"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Timeline]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
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
        if let Body::Timeline(d) = body {
            render_timeline_at(frame, area, d, opts, Utc::now().timestamp(), theme);
        }
    }
}

fn render_timeline_at(
    frame: &mut Frame,
    area: Rect,
    data: &TimelineData,
    opts: &RenderOptions,
    now: i64,
    theme: &Theme,
) {
    let prefixes: Vec<String> = data
        .events
        .iter()
        .map(|e| format_relative(e.timestamp, now))
        .collect();
    let prefix_width = prefixes
        .iter()
        .map(|p| p.chars().count())
        .max()
        .unwrap_or(0);
    let dim = Style::default().fg(theme.text_dim);

    let mut lines: Vec<Line> = Vec::with_capacity(data.events.len() * 2);
    let mut content_width: usize = 0;
    for (prefix, event) in prefixes.iter().zip(data.events.iter()) {
        let head = format!("{}{SEPARATOR}", pad_left(prefix, prefix_width));
        content_width = content_width.max(head.chars().count() + event.title.chars().count());
        lines.push(Line::from(vec![
            Span::styled(head, dim),
            Span::styled(event.title.clone(), status_style(event.status, theme)),
        ]));
        if let Some(detail) = &event.detail {
            let indent = format!("{}{SEPARATOR}", " ".repeat(prefix_width));
            content_width = content_width.max(indent.chars().count() + detail.chars().count());
            lines.push(Line::from(vec![
                Span::styled(indent, dim),
                Span::styled(detail.clone(), Style::default().fg(theme.text_secondary)),
            ]));
        }
    }

    let target = align_rect(area, content_width as u16, opts.align.as_deref());
    // Base `theme.text` so event titles without status, and any padding chars around
    // styled spans, inherit the chrome colour instead of the terminal fg.
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.text)),
        target,
    );
}

fn align_rect(area: Rect, content_width: u16, align: Option<&str>) -> Rect {
    if content_width == 0 || content_width >= area.width {
        return area;
    }
    let offset = match align {
        Some("center") => (area.width - content_width) / 2,
        Some("right") => area.width - content_width,
        _ => return area,
    };
    Rect {
        x: area.x + offset,
        y: area.y,
        width: content_width,
        height: area.height,
    }
}

fn pad_left(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(width);
    out.extend(std::iter::repeat_n(' ', width - n));
    out.push_str(s);
    out
}

fn status_style(status: Option<Status>, theme: &Theme) -> Style {
    match status {
        Some(s) => Style::default().fg(super::status_badge::status_color(s, theme)),
        None => Style::default(),
    }
}

/// Relative label for `ts` seen from `now`. Past events shrink to `Ns`/`Nm`/`Nh`/`Nd`/`Nw`;
/// anything older than ~4 weeks falls back to an absolute date so the label stays short. Future
/// events get an `in …` prefix. Same clock for past and future keeps the column width stable.
fn format_relative(ts: i64, now: i64) -> String {
    let delta = now - ts;
    if delta.abs() < 45 {
        return "now".into();
    }
    let abs = delta.abs();
    let compact = compact_delta(abs);
    match compact {
        Some(s) => {
            if delta >= 0 {
                s
            } else {
                format!("in {s}")
            }
        }
        None => format_absolute(ts, now),
    }
}

fn compact_delta(abs: i64) -> Option<String> {
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * 60;
    const DAY: i64 = 60 * 60 * 24;
    const WEEK: i64 = DAY * 7;
    if abs < HOUR {
        Some(format!("{}m", (abs + MIN / 2) / MIN))
    } else if abs < DAY {
        Some(format!("{}h", abs / HOUR))
    } else if abs < WEEK {
        Some(format!("{}d", abs / DAY))
    } else if abs < WEEK * 4 {
        Some(format!("{}w", abs / WEEK))
    } else {
        None
    }
}

fn format_absolute(ts: i64, now: i64) -> String {
    let Some(event) = Utc.timestamp_opt(ts, 0).single() else {
        return "—".into();
    };
    let Some(now_dt) = Utc.timestamp_opt(now, 0).single() else {
        return event.format("%Y-%m-%d").to_string();
    };
    if event.year() == now_dt.year() {
        event.format("%b %-d").to_string()
    } else {
        event.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Body, Payload, Status, TimelineData, TimelineEvent};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    fn render_at(data: &TimelineData, now: i64, w: u16, h: u16) -> Buffer {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        terminal
            .draw(|f| render_timeline_at(f, f.area(), data, &RenderOptions::default(), now, &theme))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn format_relative_seconds_is_now() {
        assert_eq!(format_relative(1_000, 1_030), "now");
    }

    #[test]
    fn format_relative_minutes() {
        assert_eq!(format_relative(0, 120), "2m");
    }

    #[test]
    fn format_relative_minutes_rounds_half_up() {
        assert_eq!(format_relative(0, 45), "1m");
    }

    #[test]
    fn format_relative_hours() {
        assert_eq!(format_relative(0, 3 * 3600), "3h");
    }

    #[test]
    fn format_relative_days() {
        assert_eq!(format_relative(0, 2 * 86_400), "2d");
    }

    #[test]
    fn format_relative_weeks() {
        assert_eq!(format_relative(0, 8 * 86_400), "1w");
    }

    #[test]
    fn format_relative_falls_back_to_absolute_date_after_four_weeks() {
        let now = Utc
            .with_ymd_and_hms(2026, 4, 22, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let ts = now - 60 * 86_400; // → 2026-02-21
        assert_eq!(format_relative(ts, now), "Feb 21");
    }

    #[test]
    fn format_relative_absolute_includes_year_across_boundary() {
        // now = 2026-02-15, event 2025-11-20 → different year, ISO date.
        let now = Utc
            .with_ymd_and_hms(2026, 2, 15, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let ts = Utc
            .with_ymd_and_hms(2025, 11, 20, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(format_relative(ts, now), "2025-11-20");
    }

    #[test]
    fn format_relative_future_gets_in_prefix() {
        assert_eq!(format_relative(3_600, 0), "in 1h");
    }

    #[test]
    fn renders_event_title_with_relative_prefix() {
        let data = TimelineData {
            events: vec![TimelineEvent {
                timestamp: 0,
                title: "merged #42".into(),
                detail: None,
                status: None,
            }],
        };
        let buf = render_at(&data, 3 * 3600, 40, 1);
        let line = row_text(&buf, 0);
        assert!(line.contains("3h"), "missing relative prefix: {line:?}");
        assert!(line.contains("merged #42"), "missing title: {line:?}");
    }

    #[test]
    fn detail_lands_on_next_row_indented() {
        let data = TimelineData {
            events: vec![TimelineEvent {
                timestamp: 0,
                title: "opened #41".into(),
                detail: Some("widget catalog".into()),
                status: None,
            }],
        };
        let buf = render_at(&data, 3 * 3600, 40, 2);
        let row0 = row_text(&buf, 0);
        let row1 = row_text(&buf, 1);
        assert!(row0.contains("opened #41"));
        assert!(row1.contains("widget catalog"));
        // Detail row starts with spaces, not with "3h".
        assert!(!row1.trim_start().starts_with("3h"));
    }

    #[test]
    fn status_colours_the_title() {
        let data = TimelineData {
            events: vec![TimelineEvent {
                timestamp: 0,
                title: "build failed".into(),
                detail: None,
                status: Some(Status::Error),
            }],
        };
        let buf = render_at(&data, 60, 40, 1);
        // Title follows the separator " │ " — scan for the 'b' of "build".
        let row = row_text(&buf, 0);
        let col = row.find("build").expect("title should appear") as u16;
        assert_eq!(
            buf.cell((col, 0)).unwrap().fg,
            Theme::default().status_error
        );
    }

    #[test]
    fn prefixes_share_a_column_width() {
        let data = TimelineData {
            events: vec![
                TimelineEvent {
                    timestamp: 0,
                    title: "old".into(),
                    detail: None,
                    status: None,
                },
                TimelineEvent {
                    timestamp: 10 * 86_400 - 60, // still under 28d → "1w"
                    title: "recent".into(),
                    detail: None,
                    status: None,
                },
            ],
        };
        // now = 10d past epoch: first event is "10d", second is "1w". Widths 3 and 2 — the
        // "1w" row should be right-padded so the " │ " separator lines up with the "10d" row.
        let buf = render_at(&data, 10 * 86_400, 40, 2);
        let row0 = row_text(&buf, 0);
        let row1 = row_text(&buf, 1);
        let col0 = row0.find('│').unwrap();
        let col1 = row1.find('│').unwrap();
        assert_eq!(col0, col1, "separators misaligned: {row0:?} vs {row1:?}");
    }

    #[test]
    fn timeline_is_the_default_renderer_for_timeline_shape() {
        // Uses real Utc::now() through the public path — we only assert the title lands, not
        // the prefix, since the relative label is clock-dependent.
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Timeline(TimelineData {
                events: vec![TimelineEvent {
                    timestamp: Utc::now().timestamp(),
                    title: "right now".into(),
                    detail: None,
                    status: None,
                }],
            }),
        };
        let registry = Registry::with_builtins();
        let buf = render_to_buffer_with_spec(&p, None, &registry, 40, 1);
        assert!(line_text(&buf, 0).contains("right now"));
    }

    #[test]
    fn timeline_renders_via_short_spec() {
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Timeline(TimelineData {
                events: vec![TimelineEvent {
                    timestamp: Utc::now().timestamp() - 120,
                    title: "x".into(),
                    detail: None,
                    status: None,
                }],
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("list_timeline".into());
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 20, 1);
        assert!(line_text(&buf, 0).contains("x"));
    }
}
