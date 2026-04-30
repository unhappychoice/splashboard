use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    widgets::{Paragraph, Wrap},
};
use tui_markdown::{Options as MdOptions, StyleSheet};

use crate::options::OptionSchema;
use crate::payload::Body;
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[
    theme::TEXT,
    theme::PANEL_TITLE,
    theme::ACCENT_EVENT,
    theme::TEXT_SECONDARY,
    theme::TEXT_DIM,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "align",
    type_hint: "\"left\" | \"center\" | \"right\"",
    required: false,
    default: Some("\"left\""),
    description: "Horizontal alignment of the rendered markdown within its cell.",
}];

/// Markdown renderer wrapping the `tui-markdown` crate. Accepts only `MarkdownTextBlock`
/// — a fetcher emitting that shape is committing to "this string is Markdown source",
/// which is a different semantic claim from `Text` ("this string should be drawn as-is")
/// and `TextBlock` ("these are independent lines"). All styled spans (headings, code,
/// links, blockquote) come from the splashboard `Theme` via [`SplashStyleSheet`] so the
/// markdown reads as one place with the rest of the splash. Syntax highlighting via
/// syntect is intentionally disabled (`tui-markdown` `default-features = false`) to keep
/// fenced code blocks on the same theme tokens.
pub struct TextMarkdownRenderer;

impl Renderer for TextMarkdownRenderer {
    fn name(&self) -> &str {
        "text_markdown"
    }
    fn description(&self) -> &'static str {
        "Wrapped markdown prose with themed headings, inline code, links, and blockquotes. Pick this when the source string carries Markdown syntax that should render as styled spans rather than literal characters."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::MarkdownTextBlock]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
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
        let Body::MarkdownTextBlock(d) = body else {
            return;
        };
        let p = Paragraph::new(parse_markdown(&d.value, theme))
            .style(Style::default().fg(theme.text))
            .alignment(parse_align(opts.align.as_deref()))
            .wrap(Wrap { trim: false });
        frame.render_widget(p, area);
    }
    fn natural_height(
        &self,
        body: &Body,
        _opts: &RenderOptions,
        max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        let Body::MarkdownTextBlock(d) = body else {
            return 1;
        };
        wrapped_line_count(&parse_markdown(&d.value, &Theme::default()), max_width)
    }
}

fn parse_markdown<'a>(source: &'a str, theme: &Theme) -> ratatui::text::Text<'a> {
    let opts = MdOptions::new(SplashStyleSheet::from(theme));
    tui_markdown::from_str_with_options(source, &opts)
}

/// Maps `tui-markdown`'s six style hooks onto splashboard theme tokens.
///
/// - `heading(1..=3)` → `panel_title` (the coral hero) with bold / italic gradient so H1
///   reads loudest. H4-H6 fall back to `text_secondary` italic — section breaks rather
///   than headlines.
/// - `code()` → `accent_event` (cyan-teal). One token covers both inline code spans and
///   fenced code blocks (tui-markdown shares the hook); cyan-teal sits opposite the coral
///   heading so code is visually distinct without competing.
/// - `link()` → `accent_event` + underlined.
/// - `blockquote()` → `text_secondary` italic — quoted material stays muted.
/// - `heading_meta()` / `metadata_block()` → `text_dim`.
#[derive(Debug, Clone)]
struct SplashStyleSheet {
    heading: Color,
    code: Color,
    link: Color,
    secondary: Color,
    dim: Color,
}

impl SplashStyleSheet {
    fn from(theme: &Theme) -> Self {
        Self {
            heading: theme.panel_title,
            code: theme.accent_event,
            link: theme.accent_event,
            secondary: theme.text_secondary,
            dim: theme.text_dim,
        }
    }
}

impl StyleSheet for SplashStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::default()
                .fg(self.heading)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            2 => Style::default()
                .fg(self.heading)
                .add_modifier(Modifier::BOLD),
            3 => Style::default()
                .fg(self.heading)
                .add_modifier(Modifier::BOLD | Modifier::ITALIC),
            _ => Style::default()
                .fg(self.secondary)
                .add_modifier(Modifier::ITALIC),
        }
    }
    fn code(&self) -> Style {
        Style::default().fg(self.code)
    }
    fn link(&self) -> Style {
        Style::default()
            .fg(self.link)
            .add_modifier(Modifier::UNDERLINED)
    }
    fn blockquote(&self) -> Style {
        Style::default()
            .fg(self.secondary)
            .add_modifier(Modifier::ITALIC)
    }
    fn heading_meta(&self) -> Style {
        Style::default().fg(self.dim)
    }
    fn metadata_block(&self) -> Style {
        Style::default().fg(self.dim)
    }
}

fn wrapped_line_count(text: &ratatui::text::Text<'_>, width: u16) -> u16 {
    let w = width.max(1) as usize;
    text.lines
        .iter()
        .map(|line| {
            let cells: usize = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum();
            cells.max(1).div_ceil(w)
        })
        .sum::<usize>()
        .clamp(1, u16::MAX as usize) as u16
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::*;
    use crate::payload::{MarkdownTextBlockData, Payload, TextData};
    use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn md_payload(value: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: value.to_string(),
            }),
        }
    }

    fn spec() -> RenderSpec {
        RenderSpec::Full {
            type_name: "text_markdown".into(),
            options: RenderOptions::default(),
        }
    }

    fn render(payload: &Payload, w: u16, h: u16) -> ratatui::buffer::Buffer {
        render_to_buffer_with_spec(payload, Some(&spec()), &Registry::with_builtins(), w, h)
    }

    fn render_direct(body: &Body, opts: &RenderOptions, w: u16, h: u16) -> Buffer {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        let renderer = TextMarkdownRenderer;
        let theme = Theme::default();
        let registry = Registry::with_builtins();
        terminal
            .draw(|frame| renderer.render(frame, frame.area(), body, opts, &theme, &registry))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Find the first cell on `row` whose printable symbol contains `needle` and return its
    /// foreground colour. Lets style-attribution tests assert colour without coupling to
    /// exact x-positions (heading prefixes, list markers, padding all shift cells).
    fn fg_at(buf: &ratatui::buffer::Buffer, row: u16, needle: char) -> Color {
        let area = buf.area;
        for x in 0..area.width {
            let cell = buf.cell((x, row)).unwrap();
            if cell.symbol().contains(needle) {
                return cell.style().fg.unwrap_or(Color::Reset);
            }
        }
        panic!("char {needle:?} not found on row {row}");
    }

    #[test]
    fn renders_plain_paragraph() {
        let buf = render(&md_payload("hello world"), 30, 5);
        assert!(line_text(&buf, 0).contains("hello world"));
    }

    #[test]
    fn renderer_exposes_catalog_metadata() {
        let renderer = TextMarkdownRenderer;
        assert_eq!(
            renderer.description(),
            "Wrapped markdown prose with themed headings, inline code, links, and blockquotes. Pick this when the source string carries Markdown syntax that should render as styled spans rather than literal characters."
        );
        assert_eq!(renderer.accepts(), &[Shape::MarkdownTextBlock]);
        assert_eq!(renderer.option_schemas()[0].name, "align");
        assert_eq!(renderer.color_keys().len(), COLOR_KEYS.len());
        assert_eq!(renderer.color_keys()[0].name, theme::TEXT.name);
        assert_eq!(renderer.color_keys()[4].name, theme::TEXT_DIM.name);
    }

    #[test]
    fn renders_heading_then_body() {
        // tui-markdown keeps `#` in the heading line and inserts a blank gap before the body.
        let buf = render(&md_payload("# Welcome\nbody line"), 30, 5);
        assert!(line_text(&buf, 0).contains("Welcome"));
        assert!(line_text(&buf, 2).contains("body line"));
    }

    #[test]
    fn renders_bullet_list() {
        let buf = render(&md_payload("- one\n- two"), 30, 5);
        assert!(line_text(&buf, 0).contains("one"));
        assert!(line_text(&buf, 1).contains("two"));
    }

    #[test]
    fn natural_height_grows_with_lines() {
        let r = TextMarkdownRenderer;
        let registry = Registry::with_builtins();
        let opts = RenderOptions::default();
        let one = r.natural_height(&md_payload("hello").body, &opts, 30, &registry);
        let many = r.natural_height(
            &md_payload("# Heading\nline 1\nline 2\nline 3").body,
            &opts,
            30,
            &registry,
        );
        assert!(many > one);
    }

    #[test]
    fn natural_height_defaults_to_one_for_wrong_shape() {
        let renderer = TextMarkdownRenderer;
        let registry = Registry::with_builtins();
        assert_eq!(
            renderer.natural_height(
                &Body::Text(TextData {
                    value: "plain".into(),
                }),
                &RenderOptions::default(),
                30,
                &registry,
            ),
            1
        );
    }

    #[test]
    fn does_not_panic_on_narrow_area() {
        let _ = render(
            &md_payload("# Big heading that overflows narrow area"),
            5,
            3,
        );
    }

    #[test]
    fn rejects_plain_text_shape() {
        // Markdown source MUST come in via `MarkdownTextBlock`. A `Text` payload routed to
        // this renderer is a misconfiguration — the dispatcher draws an in-band error
        // instead of letting tui-markdown silently render the plain text.
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: "**bold** but plain".into(),
            }),
        };
        let buf = render(&p, 40, 3);
        assert!(line_text(&buf, 0).to_lowercase().contains("cannot display"));
    }

    #[test]
    fn direct_render_ignores_non_markdown_bodies() {
        let buf = render_direct(
            &Body::Text(TextData {
                value: "plain".into(),
            }),
            &RenderOptions::default(),
            20,
            2,
        );
        assert!(line_text(&buf, 0).trim().is_empty());
        assert!(line_text(&buf, 1).trim().is_empty());
    }

    #[test]
    fn heading_picks_up_panel_title_color() {
        let theme = Theme::default();
        let buf = render(&md_payload("# Welcome"), 30, 3);
        assert_eq!(fg_at(&buf, 0, 'W'), theme.panel_title);
    }

    #[test]
    fn inline_code_picks_up_accent_event_color() {
        let theme = Theme::default();
        let buf = render(&md_payload("hello `world` end"), 30, 3);
        assert_eq!(fg_at(&buf, 0, 'w'), theme.accent_event);
    }

    #[test]
    fn body_text_picks_up_text_color() {
        let theme = Theme::default();
        let buf = render(&md_payload("hello world"), 30, 3);
        assert_eq!(fg_at(&buf, 0, 'h'), theme.text);
    }

    #[test]
    fn blockquote_picks_up_text_secondary_color() {
        let theme = Theme::default();
        let buf = render(&md_payload("> quoted line"), 30, 3);
        assert_eq!(fg_at(&buf, 0, 'q'), theme.text_secondary);
    }

    #[test]
    fn h4_falls_back_to_text_secondary() {
        let theme = Theme::default();
        let buf = render(&md_payload("#### deep heading"), 30, 3);
        assert_eq!(fg_at(&buf, 0, 'd'), theme.text_secondary);
    }

    #[test]
    fn stylesheet_covers_remaining_heading_levels_and_code_family() {
        let theme = Theme::default();
        let styles = SplashStyleSheet::from(&theme);
        assert_eq!(styles.heading(2).fg, Some(theme.panel_title));
        assert!(styles.heading(2).add_modifier.contains(Modifier::BOLD));
        assert_eq!(styles.heading(3).fg, Some(theme.panel_title));
        assert!(styles.heading(3).add_modifier.contains(Modifier::ITALIC));
        assert_eq!(styles.heading(6).fg, Some(theme.text_secondary));
        assert!(styles.heading(6).add_modifier.contains(Modifier::ITALIC));
        assert_eq!(styles.code().fg, Some(theme.accent_event));
        assert_eq!(styles.link().fg, Some(theme.accent_event));
        assert!(styles.link().add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(styles.blockquote().fg, Some(theme.text_secondary));
        assert!(styles.blockquote().add_modifier.contains(Modifier::ITALIC));
        assert_eq!(styles.heading_meta().fg, Some(theme.text_dim));
        assert_eq!(styles.metadata_block().fg, Some(theme.text_dim));
    }

    #[test]
    fn parse_align_supports_center_right_and_fallback() {
        assert_eq!(parse_align(Some("center")), Alignment::Center);
        assert_eq!(parse_align(Some("right")), Alignment::Right);
        assert_eq!(parse_align(Some("bogus")), Alignment::Left);
        assert_eq!(parse_align(None), Alignment::Left);
    }
}
