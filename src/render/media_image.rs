use std::io::IsTerminal;
use std::sync::OnceLock;

use image::DynamicImage;
use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};
use ratatui_image::{Resize, StatefulImage, picker::Picker};

use crate::options::OptionSchema;
use crate::payload::{Body, ImageData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal placement within the available cell when the image is narrower (after `max_width` / `fit = \"contain\"`).",
    },
    OptionSchema {
        name: "fit",
        type_hint: "\"contain\" | \"cover\" | \"stretch\"",
        required: false,
        default: Some("\"contain\""),
        description: "How the image is sized into its box. `contain` preserves aspect; `cover` fills the box and crops overflow; `stretch` warps to fit.",
    },
    OptionSchema {
        name: "max_width",
        type_hint: "cells (u16)",
        required: false,
        default: Some("area width"),
        description: "Upper bound on image width in cells.",
    },
    OptionSchema {
        name: "max_height",
        type_hint: "cells (u16)",
        required: false,
        default: Some("area height"),
        description: "Upper bound on image height in cells.",
    },
    OptionSchema {
        name: "padding",
        type_hint: "cells (u16)",
        required: false,
        default: Some("0"),
        description: "Uniform inset inside the image cell (all four sides). Applied before alignment and sizing.",
    },
];

pub struct MediaImageRenderer;

impl Renderer for MediaImageRenderer {
    fn name(&self) -> &str {
        "media_image"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Image]
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
        if let Body::Image(d) = body {
            render_image_payload(frame, area, d, opts, theme);
        }
    }
}

fn render_image_payload(
    frame: &mut Frame,
    area: Rect,
    data: &ImageData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let target = compute_target(area, opts);
    match render_image(frame, target, &data.path, opts) {
        Ok(()) => {}
        Err(msg) => frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(theme.text)),
            target,
        ),
    }
}

fn render_image(
    frame: &mut Frame,
    area: Rect,
    path: &str,
    opts: &RenderOptions,
) -> Result<(), String> {
    let img = load_image(path)?;
    let mut protocol = picker().new_resize_protocol(img);
    let widget = StatefulImage::default().resize(resize_mode(opts.fit.as_deref()));
    frame.render_stateful_widget(widget, area, &mut protocol);
    Ok(())
}

/// Apply padding, `max_width`/`max_height`, and alignment to produce the final cell rect the
/// image protocol draws into. Padding shrinks first — it's "inner margin" the whole layout sees
/// as empty — then max_* caps the body, then alignment places that box inside what remains of
/// the original area.
fn compute_target(area: Rect, opts: &RenderOptions) -> Rect {
    let padded = apply_padding(area, opts.padding.unwrap_or(0));
    let capped_w = opts.max_width.map_or(padded.width, |m| m.min(padded.width));
    let capped_h = opts
        .max_height
        .map_or(padded.height, |m| m.min(padded.height));
    let x_offset = match opts.align.as_deref() {
        Some("center") => padded.width.saturating_sub(capped_w) / 2,
        Some("right") => padded.width.saturating_sub(capped_w),
        _ => 0,
    };
    Rect {
        x: padded.x + x_offset,
        y: padded.y,
        width: capped_w,
        height: capped_h,
    }
}

fn apply_padding(area: Rect, padding: u16) -> Rect {
    let pad_x = padding.min(area.width / 2);
    let pad_y = padding.min(area.height / 2);
    Rect {
        x: area.x + pad_x,
        y: area.y + pad_y,
        width: area.width.saturating_sub(pad_x * 2),
        height: area.height.saturating_sub(pad_y * 2),
    }
}

fn resize_mode(fit: Option<&str>) -> Resize {
    match fit {
        Some("cover") => Resize::Crop(None),
        Some("stretch") => Resize::Scale(None),
        _ => Resize::Fit(None),
    }
}

fn picker() -> Picker {
    static CACHED: OnceLock<Picker> = OnceLock::new();
    *CACHED.get_or_init(detect_picker)
}

fn detect_picker() -> Picker {
    if std::io::stdin().is_terminal() {
        Picker::from_query_stdio().unwrap_or_else(|_| Picker::from_fontsize((8, 16)))
    } else {
        Picker::from_fontsize((8, 16))
    }
}

fn load_image(path: &str) -> Result<DynamicImage, String> {
    image::ImageReader::open(path)
        .map_err(|e| format!("[image: {e}]"))?
        .decode()
        .map_err(|e| format!("[image decode: {e}]"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{ImageData, Payload};
    use crate::render::test_utils::{line_text, render_to_buffer};

    fn payload(path: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Image(ImageData { path: path.into() }),
        }
    }

    #[test]
    fn falls_back_to_text_when_path_missing() {
        let buf = render_to_buffer(&payload("/does/not/exist.png"), 40, 5);
        let content: String = (0..5)
            .map(|y| line_text(&buf, y))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(content.contains("image"));
    }

    #[test]
    fn padding_shrinks_area_uniformly() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 10,
        };
        let opts = RenderOptions {
            padding: Some(2),
            ..RenderOptions::default()
        };
        let out = compute_target(area, &opts);
        assert_eq!(
            out,
            Rect {
                x: 2,
                y: 2,
                width: 16,
                height: 6
            }
        );
    }

    #[test]
    fn max_width_caps_and_center_align_positions_box() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 10,
        };
        let opts = RenderOptions {
            max_width: Some(10),
            align: Some("center".into()),
            ..RenderOptions::default()
        };
        let out = compute_target(area, &opts);
        assert_eq!(out.width, 10);
        assert_eq!(out.x, 10);
    }

    #[test]
    fn max_height_caps_below_area_height() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 20,
            height: 20,
        };
        let opts = RenderOptions {
            max_height: Some(5),
            ..RenderOptions::default()
        };
        let out = compute_target(area, &opts);
        assert_eq!(out.height, 5);
    }

    #[test]
    fn right_align_pushes_box_to_far_edge() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 10,
        };
        let opts = RenderOptions {
            max_width: Some(10),
            align: Some("right".into()),
            ..RenderOptions::default()
        };
        let out = compute_target(area, &opts);
        assert_eq!(out.x, 20);
        assert_eq!(out.width, 10);
    }

    #[test]
    fn resize_mode_maps_fit_option() {
        assert!(matches!(resize_mode(Some("cover")), Resize::Crop(_)));
        assert!(matches!(resize_mode(Some("stretch")), Resize::Scale(_)));
        assert!(matches!(resize_mode(Some("contain")), Resize::Fit(_)));
        assert!(matches!(resize_mode(None), Resize::Fit(_)));
    }
}
