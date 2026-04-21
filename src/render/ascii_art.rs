use ratatui::{Frame, layout::Rect, widgets::Paragraph};
use tui_big_text::{BigText, PixelSize};

use crate::payload::{Body, LinesData};

use super::{RenderOptions, Renderer, Shape};

/// ASCII-art text rendering over the `Lines` shape. The `style` option selects the visual
/// treatment; additional sub-options refine the chosen style.
///
/// - `style = "blocks"` (default): half-block glyphs via `tui-big-text`. Sub-option
///   `pixel_size` = "full" | "quadrant" | "sextant"; unset picks by area height.
/// - `style = "figlet"`: classic FIGlet ASCII art via `figlet-rs`. No sub-options yet
///   (custom fonts land when we make a config decision on font bundling).
pub struct AsciiArtRenderer;

impl Renderer for AsciiArtRenderer {
    fn name(&self) -> &str {
        "ascii_art"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Lines]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, opts: &RenderOptions) {
        if let Body::Lines(d) = body {
            match opts.style.as_deref() {
                Some("figlet") => render_figlet(frame, area, d),
                _ => render_blocks(frame, area, d, opts),
            }
        }
    }
}

fn render_blocks(frame: &mut Frame, area: Rect, data: &LinesData, opts: &RenderOptions) {
    let pixel_size = opts
        .pixel_size
        .as_deref()
        .and_then(parse_pixel_size)
        .unwrap_or_else(|| pick_pixel_size(area));
    let lines: Vec<_> = data.lines.iter().map(|s| s.clone().into()).collect();
    let big = BigText::builder()
        .pixel_size(pixel_size)
        .lines(lines)
        .build();
    frame.render_widget(big, area);
}

fn render_figlet(frame: &mut Frame, area: Rect, data: &LinesData) {
    let rendered = data
        .lines
        .iter()
        .map(|l| figletify(l))
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(Paragraph::new(rendered), area);
}

fn figletify(text: &str) -> String {
    let Ok(font) = figlet_rs::FIGfont::standard() else {
        return text.to_string();
    };
    font.convert(text)
        .map(|f| f.to_string())
        .unwrap_or_else(|| text.to_string())
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
    use crate::payload::{LinesData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(text: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData {
                lines: vec![text.into()],
            }),
        }
    }

    #[test]
    fn default_style_uses_blocks() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("ascii_art".into());
        let _ = render_to_buffer_with_spec(&payload("12:34"), Some(&spec), &registry, 40, 10);
    }

    #[test]
    fn figlet_style_routes_through_figlet_rs() {
        // `type = "ascii_art", style = "figlet"` should produce ASCII-art output without
        // panicking — we don't assert specific glyphs since figlet fonts are ornate.
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W = toml::from_str(r#"render = { type = "ascii_art", style = "figlet" }"#).unwrap();
        let registry = Registry::with_builtins();
        let _ = render_to_buffer_with_spec(&payload("hi"), Some(&w.render), &registry, 60, 10);
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
