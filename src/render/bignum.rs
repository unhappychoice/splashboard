use ratatui::{Frame, layout::Rect};
use tui_big_text::{BigText, PixelSize};

use crate::payload::{Body, LinesData};

use super::{RenderOptions, Renderer, Shape};

/// Big-text renderer over the `Lines` shape. Uses `tui-big-text` for the actual glyphs. Takes
/// an optional `pixel_size` ("full" / "quadrant" / "sextant"); unspecified picks by area height
/// so small frames still display something readable.
pub struct BignumTuiRenderer;

impl Renderer for BignumTuiRenderer {
    fn name(&self) -> &str {
        "bignum_tui"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Lines]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, opts: &RenderOptions) {
        if let Body::Lines(d) = body {
            render_bignum(frame, area, d, opts);
        }
    }
}

fn render_bignum(frame: &mut Frame, area: Rect, data: &LinesData, opts: &RenderOptions) {
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
    fn renders_without_panicking() {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("bignum_tui".into());
        let _ = render_to_buffer_with_spec(&payload("12:34"), Some(&spec), &registry, 40, 10);
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
