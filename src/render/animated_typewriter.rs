use std::sync::OnceLock;
use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    widgets::Paragraph,
};

use crate::options::OptionSchema;
use crate::payload::{Body, TextData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const DEFAULT_CHARS_PER_SECOND: f64 = 40.0;

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "align",
    type_hint: "\"left\" | \"center\" | \"right\"",
    required: false,
    default: Some("\"left\""),
    description: "Horizontal alignment of the revealed text within its cell.",
}];

/// Typewriter-style progressive reveal over the `Text` shape. Each frame shows
/// `elapsed * chars_per_second` characters of the payload, truncating to whatever has "typed"
/// by now. Only animates inside a render loop — on cached fast-path draws it appears fully
/// typed, which is intentional (we don't slow repeat invocations for effect).
///
/// Accepts `Text` only. A typewriter reveal over a multi-line block (release notes, commit
/// list) is rarely what the user wants, and the fetcher choosing `Text` vs `TextBlock` signals
/// intent clearly — single-sentence greetings are the canonical use.
pub struct AnimatedTypewriterRenderer;

impl Renderer for AnimatedTypewriterRenderer {
    fn name(&self) -> &str {
        "animated_typewriter"
    }
    fn description(&self) -> &'static str {
        "Reveals the text one character at a time at roughly 40 chars/sec, as if someone is typing it live. Accepts `Text` only; best for short single-line greetings rather than multi-line blocks."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn animates(&self) -> bool {
        true
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
        if let Body::Text(d) = body {
            render_typewriter(frame, area, d, opts, theme);
        }
    }
}

fn render_typewriter(
    frame: &mut Frame,
    area: Rect,
    data: &TextData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let total = data.value.chars().count();
    let budget = chars_revealed(total);
    let revealed: String = data.value.chars().take(budget).collect();
    let p = Paragraph::new(revealed)
        .style(Style::default().fg(theme.text))
        .alignment(parse_align(opts.align.as_deref()));
    frame.render_widget(p, area);
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

/// Characters that should be visible given the time elapsed since render loop start.
fn chars_revealed(total: usize) -> usize {
    let elapsed = process_start().elapsed().as_secs_f64();
    let budget = (elapsed * DEFAULT_CHARS_PER_SECOND).floor() as usize;
    budget.min(total)
}

/// First-caller-wins timestamp. Shared across renderer invocations so a multi-frame loop sees
/// monotonically increasing elapsed time.
fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::*;
    use crate::payload::{ErrorData, ErrorKind, Payload};
    use crate::render::{
        RenderSpec,
        test_utils::{line_text, render_to_buffer_with_spec},
    };

    fn reveal(text: &str, budget: usize) -> String {
        text.chars().take(budget).collect()
    }

    fn text_payload(value: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: value.to_string(),
            }),
        }
    }

    fn render_direct(body: &Body, opts: &RenderOptions, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        let registry = Registry::with_builtins();
        let renderer = AnimatedTypewriterRenderer;
        terminal
            .draw(|f| renderer.render(f, f.area(), body, opts, &theme, &registry))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn wait_until_revealed(total: usize) {
        let deadline = Instant::now() + Duration::from_millis(250);
        while chars_revealed(total) < total && Instant::now() < deadline {
            sleep(Duration::from_millis(5));
        }
        assert_eq!(chars_revealed(total), total);
    }

    #[test]
    fn reveal_truncates_at_budget() {
        assert_eq!(reveal("hello", 3), "hel");
        assert_eq!(reveal("hello", 5), "hello");
        assert_eq!(reveal("hello", 99), "hello");
    }

    #[test]
    fn empty_budget_yields_empty_string() {
        assert_eq!(reveal("x", 0), "");
    }

    #[test]
    fn parse_align_supports_center_right_and_fallback() {
        assert_eq!(parse_align(Some("center")), Alignment::Center);
        assert_eq!(parse_align(Some("right")), Alignment::Right);
        assert_eq!(parse_align(Some("bogus")), Alignment::Left);
        assert_eq!(parse_align(None), Alignment::Left);
    }

    #[test]
    fn chars_revealed_clamps_to_total() {
        assert_eq!(chars_revealed(0), 0);
        wait_until_revealed(1);
        assert_eq!(chars_revealed(1), 1);
    }

    #[test]
    fn centered_render_reveals_text_once_budget_catches_up() {
        wait_until_revealed(2);
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Full {
            type_name: "animated_typewriter".into(),
            options: RenderOptions {
                align: Some("center".into()),
                ..RenderOptions::default()
            },
        };
        let buf = render_to_buffer_with_spec(&text_payload("OK"), Some(&spec), &registry, 6, 1);
        assert_eq!(line_text(&buf, 0), "  OK  ");
    }

    #[test]
    fn renderer_ignores_non_text_bodies() {
        let body = Body::Error(ErrorData {
            kind: ErrorKind::Fetch,
            message: "boom".into(),
            detail: None,
        });
        let buf = render_direct(&body, &RenderOptions::default(), 8, 1);
        assert_eq!(line_text(&buf, 0), "        ");
    }

    #[test]
    fn renderer_exposes_docs_metadata() {
        let renderer = AnimatedTypewriterRenderer;
        let schemas = renderer.option_schemas();
        let colors = renderer.color_keys();

        assert_eq!(renderer.name(), "animated_typewriter");
        assert!(renderer.animates());
        assert!(renderer.description().contains("40 chars/sec"));
        assert_eq!(renderer.accepts(), &[Shape::Text]);
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "align");
        assert_eq!(schemas[0].type_hint, "\"left\" | \"center\" | \"right\"");
        assert_eq!(schemas[0].default, Some("\"left\""));
        assert_eq!(colors.len(), 1);
        assert_eq!(colors[0].name, "text");
    }
}
