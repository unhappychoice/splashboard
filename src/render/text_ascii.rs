use figlet_rs::FIGlet;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::Paragraph,
};
use tui_big_text::{BigText, PixelSize};

use crate::options::OptionSchema;
use crate::payload::{Body, TextData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Options {
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub pixel_size: Option<String>,
}

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT, theme::PANEL_TITLE];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "style",
        type_hint: "\"blocks\" | \"figlet\"",
        required: false,
        default: Some("\"blocks\""),
        description: "Visual treatment. `blocks` uses half-block glyphs via `tui-big-text`; `figlet` uses the classic FIGlet engine.",
    },
    OptionSchema {
        name: "pixel_size",
        type_hint: "\"full\" | \"quadrant\" | \"sextant\"",
        required: false,
        default: Some("adaptive (picked by area height)"),
        description: "Glyph density for `style = \"blocks\"`. Ignored when `style = \"figlet\"`.",
    },
    OptionSchema {
        name: "font",
        type_hint: "\"standard\" | \"small\" | \"big\" | \"banner\" | \"ansi_shadow\" | \"slant\" | \"isometric\" | \"doom\"",
        required: false,
        default: Some("\"standard\""),
        description: "FIGlet font when `style = \"figlet\"`. Unknown names fall back to `standard`. `ansi_shadow` and `isometric` use Unicode box-drawing characters; `slant` is the classic italic; `doom` is a bold block face.",
    },
    OptionSchema {
        name: "color",
        type_hint: "theme token name (e.g. \"panel_title\", \"text\")",
        required: false,
        default: Some("\"text\""),
        description: "Foreground colour token. Any `[theme]` key name is accepted; unknown names fall back to the default.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the rendered text within its cell.",
    },
];

const FONT_STANDARD: &str = include_str!("figlet_fonts/standard.flf");
const FONT_SMALL: &str = include_str!("figlet_fonts/small.flf");
const FONT_BIG: &str = include_str!("figlet_fonts/big.flf");
const FONT_BANNER: &str = include_str!("figlet_fonts/banner.flf");
const FONT_ANSI_SHADOW: &str = include_str!("figlet_fonts/ansi_shadow.flf");
const FONT_SLANT: &str = include_str!("figlet_fonts/slant.flf");
const FONT_ISOMETRIC: &str = include_str!("figlet_fonts/isometric1.flf");
const FONT_DOOM: &str = include_str!("figlet_fonts/doom.flf");

/// ASCII-art text rendering over the `Text` shape. The `style` option selects the visual
/// treatment; additional sub-options refine the chosen style.
///
/// Accepts `Text` only — big glyphs span multiple terminal rows, so multi-line input would
/// blow past any reasonable widget height. Fetchers that want big-text output must emit a
/// single-string `Text` payload; multi-line fetchers should render via `text_plain` / `list_plain`.
///
/// - `style = "blocks"` (default): half-block glyphs via `tui-big-text`. Sub-option
///   `pixel_size` = "full" | "quadrant" | "sextant"; unset picks by area height.
/// - `style = "figlet"`: classic FIGlet ASCII art via `figlet-rs`. No sub-options yet
///   (custom fonts land when we make a config decision on font bundling).
pub struct TextAsciiRenderer;

impl Renderer for TextAsciiRenderer {
    fn name(&self) -> &str {
        "text_ascii"
    }
    fn description(&self) -> &'static str {
        "Chunky multi-row big-text via half-block glyphs (`style = \"blocks\"`) or classic FIGlet ASCII art (`style = \"figlet\"`). The hero treatment for short strings like a clock or greeting; use `text_plain` for anything multi-line."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Text]
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
        if let Body::Text(d) = body {
            let specific: Options = opts.parse_specific();
            match opts.style.as_deref() {
                Some("figlet") => render_figlet(frame, area, d, opts, &specific, theme),
                _ => render_blocks(frame, area, d, opts, &specific, theme),
            }
        }
    }
    fn natural_height(
        &self,
        body: &Body,
        opts: &RenderOptions,
        max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        let Body::Text(d) = body else {
            return 1;
        };
        let specific: Options = opts.parse_specific();
        match opts.style.as_deref() {
            Some("figlet") => {
                // u16::MAX lets wrap always succeed — we want the true row count
                // given the width, not the height-gated one render() uses.
                let rendered =
                    figlet_wrapped(&d.value, specific.font.as_deref(), max_width, u16::MAX);
                rendered.lines().count().clamp(1, u16::MAX as usize) as u16
            }
            _ => blocks_height(specific.pixel_size.as_deref()),
        }
    }
}

fn blocks_height(pixel_size: Option<&str>) -> u16 {
    match parse_pixel_size(pixel_size.unwrap_or("quadrant")) {
        Some(PixelSize::Full) => 8,
        Some(PixelSize::Quadrant) => 4,
        Some(PixelSize::Sextant) => 3,
        _ => 4,
    }
}

fn render_blocks(
    frame: &mut Frame,
    area: Rect,
    data: &TextData,
    opts: &RenderOptions,
    specific: &Options,
    theme: &Theme,
) {
    let pixel_size = specific
        .pixel_size
        .as_deref()
        .and_then(parse_pixel_size)
        .unwrap_or_else(|| pick_pixel_size(area));
    let natural_width = blocks_width(&data.value, pixel_size);
    let target = align_rect(area, natural_width, opts.align.as_deref());
    let big = BigText::builder()
        .pixel_size(pixel_size)
        .style(Style::default().fg(resolve_color(opts, theme)))
        .lines(vec![data.value.clone().into()])
        .build();
    frame.render_widget(big, target);
}

fn render_figlet(
    frame: &mut Frame,
    area: Rect,
    data: &TextData,
    opts: &RenderOptions,
    specific: &Options,
    theme: &Theme,
) {
    let rendered = figlet_wrapped(
        &data.value,
        specific.font.as_deref(),
        area.width,
        area.height,
    );
    let p = Paragraph::new(rendered)
        .style(Style::default().fg(resolve_color(opts, theme)))
        .alignment(parse_alignment(opts.align.as_deref()));
    frame.render_widget(p, area);
}

/// Figlet-render `text` and word-wrap the output when it overflows `max_width`
/// *and* the wrapped version still fits inside `max_height`. A hero cell that's
/// too short for a 2-block stack (typical 7 + 7 = 14 rows for ansi_shadow)
/// falls back to the single joined block, which ratatui's `Paragraph` clips to
/// area width — better a partially visible figlet than a silently-empty cell.
fn figlet_wrapped(text: &str, font_name: Option<&str>, max_width: u16, max_height: u16) -> String {
    let font = load_font(font_name);
    let render = |s: &str| font.convert(s).map(|f| f.to_string());
    let Some(full) = render(text) else {
        return text.to_string();
    };
    if block_width(&full) <= max_width as usize || !text.contains(' ') {
        return full;
    }
    let mut blocks: Vec<String> = Vec::new();
    let mut line_words: Vec<&str> = Vec::new();
    for word in text.split_whitespace() {
        line_words.push(word);
        let candidate = line_words.join(" ");
        let fits = render(&candidate).is_some_and(|r| block_width(&r) <= max_width as usize);
        if fits {
            continue;
        }
        line_words.pop();
        if !line_words.is_empty()
            && let Some(rendered) = render(&line_words.join(" "))
        {
            blocks.push(rendered);
        }
        line_words.clear();
        line_words.push(word);
    }
    if !line_words.is_empty()
        && let Some(rendered) = render(&line_words.join(" "))
    {
        blocks.push(rendered);
    }
    if blocks.is_empty() {
        return full;
    }
    let joined = blocks.join("\n");
    if joined.lines().count() as u16 <= max_height {
        joined
    } else {
        full
    }
}

/// Cell width of a multi-line figlet block = max char-count across all lines.
/// Figlet fonts emit same-width lines per glyph, so max == any-line width, but
/// we walk them to be safe against trailing padding differences.
fn block_width(block: &str) -> usize {
    block.lines().map(|l| l.chars().count()).max().unwrap_or(0)
}

fn resolve_color(opts: &RenderOptions, theme: &Theme) -> Color {
    match opts.color.as_deref() {
        Some(name) => theme::token_color(theme, name).unwrap_or(theme.text),
        None => theme.text,
    }
}

/// Natural width (in terminal cells) of a tui-big-text block string at the given pixel size.
/// Based on the 8x8 base font; `pixels_per_cell` compresses each axis. Only the three variants
/// we recognise in `parse_pixel_size` are listed — anything else falls back to a conservative
/// default.
fn blocks_width(text: &str, pixel_size: PixelSize) -> u16 {
    let chars = text.chars().count() as u16;
    let glyph_cols = match pixel_size {
        PixelSize::Full => 8,
        PixelSize::Quadrant => 4,
        PixelSize::Sextant => 4,
        _ => 4,
    };
    chars.saturating_mul(glyph_cols)
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

fn parse_alignment(s: Option<&str>) -> ratatui::layout::Alignment {
    match s {
        Some("center") => ratatui::layout::Alignment::Center,
        Some("right") => ratatui::layout::Alignment::Right,
        _ => ratatui::layout::Alignment::Left,
    }
}

fn load_font(name: Option<&str>) -> FIGlet {
    let source = match name.unwrap_or("standard") {
        "small" => FONT_SMALL,
        "big" => FONT_BIG,
        "banner" => FONT_BANNER,
        "ansi_shadow" => FONT_ANSI_SHADOW,
        "slant" => FONT_SLANT,
        "isometric" => FONT_ISOMETRIC,
        "doom" => FONT_DOOM,
        _ => FONT_STANDARD,
    };
    FIGlet::from_content(source).unwrap_or_else(|_| FIGlet::standard().expect("standard font"))
}

fn parse_pixel_size(s: &str) -> Option<PixelSize> {
    match s {
        "full" => Some(PixelSize::Full),
        "quadrant" => Some(PixelSize::Quadrant),
        "sextant" => Some(PixelSize::Sextant),
        _ => None,
    }
}

fn pick_pixel_size(area: Rect) -> PixelSize {
    if area.height >= 8 {
        PixelSize::Full
    } else if area.height >= 4 {
        PixelSize::Quadrant
    } else {
        PixelSize::Sextant
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Payload, TextData};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(text: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: text.into() }),
        }
    }

    #[test]
    fn default_style_uses_blocks() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("text_ascii".into());
        let _ = render_to_buffer_with_spec(&payload("12:34"), Some(&spec), &registry, 40, 10);
    }

    #[test]
    fn figlet_style_routes_through_figlet_rs() {
        // `type = "text_ascii", style = "figlet"` should produce ASCII-art output without
        // panicking — we don't assert specific glyphs since figlet fonts are ornate.
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "text_ascii", style = "figlet" }"#).unwrap();
        let registry = Registry::with_builtins();
        let _ = render_to_buffer_with_spec(&payload("hi"), Some(&w.render), &registry, 60, 10);
    }

    #[test]
    fn figlet_fonts_load_without_panicking() {
        // Every bundled font parses through figlet_rs and converts a sample string.
        for name in [
            "standard",
            "small",
            "big",
            "banner",
            "ansi_shadow",
            "slant",
            "isometric",
            "doom",
        ] {
            let text = figlet_wrapped("hi", Some(name), 200, 200);
            assert!(!text.is_empty(), "font {name} produced empty output");
        }
    }

    #[test]
    fn unknown_font_falls_back_to_standard() {
        let fallback = figlet_wrapped("hi", Some("no-such-font"), 200, 200);
        let std_out = figlet_wrapped("hi", Some("standard"), 200, 200);
        assert_eq!(fallback, std_out);
    }

    #[test]
    fn figlet_wraps_multiword_hero_to_fit_width() {
        // "Windows Terminal" at ansi_shadow is ~100 cells wide on one line.
        // At a 60-cell width with enough vertical room, each word lands on its
        // own 7-row block — so the wrapped output is taller than the joined.
        let joined = figlet_wrapped("Windows Terminal", Some("ansi_shadow"), 200, 200)
            .lines()
            .count();
        let wrapped = figlet_wrapped("Windows Terminal", Some("ansi_shadow"), 60, 200)
            .lines()
            .count();
        assert!(wrapped > joined, "expected wrap to produce more lines");
    }

    #[test]
    fn figlet_skips_wrap_when_no_room_for_stack() {
        // Same narrow width but only 7 rows tall — the 2-block stack won't fit
        // vertically so we return the single joined block (ratatui clips the
        // overflow, less lossy than an empty cell).
        let joined = figlet_wrapped("Windows Terminal", Some("ansi_shadow"), 200, 200);
        let wrapped = figlet_wrapped("Windows Terminal", Some("ansi_shadow"), 60, 7);
        assert_eq!(wrapped.lines().count(), joined.lines().count());
    }

    #[test]
    fn figlet_keeps_short_text_on_one_block() {
        let rendered = figlet_wrapped("Fri 24", Some("ansi_shadow"), 100, 200);
        // 7 rows tall, no wrap needed.
        assert!(rendered.lines().count() <= 8);
    }

    #[test]
    fn color_option_resolves_theme_token() {
        use crate::theme::Theme;
        let theme = Theme::default();
        let opts = RenderOptions {
            color: Some("panel_title".into()),
            ..RenderOptions::default()
        };
        assert_eq!(resolve_color(&opts, &theme), theme.panel_title);
        let opts_unknown = RenderOptions {
            color: Some("bogus".into()),
            ..RenderOptions::default()
        };
        assert_eq!(resolve_color(&opts_unknown, &theme), theme.text);
    }

    #[test]
    fn parse_pixel_size_accepts_known_values() {
        assert!(matches!(parse_pixel_size("full"), Some(PixelSize::Full)));
        assert!(matches!(
            parse_pixel_size("quadrant"),
            Some(PixelSize::Quadrant)
        ));
        assert!(matches!(
            parse_pixel_size("sextant"),
            Some(PixelSize::Sextant)
        ));
        assert!(parse_pixel_size("bogus").is_none());
    }
}
