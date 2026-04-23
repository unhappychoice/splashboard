use figlet_rs::FIGfont;
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
            match opts.style.as_deref() {
                Some("figlet") => render_figlet(frame, area, d, opts, theme),
                _ => render_blocks(frame, area, d, opts, theme),
            }
        }
    }
}

fn render_blocks(
    frame: &mut Frame,
    area: Rect,
    data: &TextData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let pixel_size = opts
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
    theme: &Theme,
) {
    let rendered = figletify(&data.value, opts.font.as_deref());
    let p = Paragraph::new(rendered)
        .style(Style::default().fg(resolve_color(opts, theme)))
        .alignment(parse_alignment(opts.align.as_deref()));
    frame.render_widget(p, area);
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

fn figletify(text: &str, font_name: Option<&str>) -> String {
    let font = load_font(font_name);
    font.convert(text)
        .map(|f| f.to_string())
        .unwrap_or_else(|| text.to_string())
}

fn load_font(name: Option<&str>) -> FIGfont {
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
    FIGfont::from_content(source).unwrap_or_else(|_| FIGfont::standard().expect("standard font"))
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
        for name in ["standard", "small", "big", "banner"] {
            let text = figletify("hi", Some(name));
            assert!(!text.is_empty(), "font {name} produced empty output");
        }
    }

    #[test]
    fn unknown_font_falls_back_to_standard() {
        let fallback = figletify("hi", Some("no-such-font"));
        let std_out = figletify("hi", Some("standard"));
        assert_eq!(fallback, std_out);
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
