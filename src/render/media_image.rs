use std::io::IsTerminal;
use std::sync::OnceLock;

use image::DynamicImage;
use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};
use ratatui_image::{StatefulImage, picker::Picker};

use crate::payload::{Body, ImageData};
use crate::theme::{self, ColorKey, Theme};

use super::{RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::TEXT];

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
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        _opts: &RenderOptions,
        theme: &Theme,
    ) {
        if let Body::Image(d) = body {
            render_image_payload(frame, area, d, theme);
        }
    }
}

fn render_image_payload(frame: &mut Frame, area: Rect, data: &ImageData, theme: &Theme) {
    match render_image(frame, area, &data.path) {
        Ok(()) => {}
        Err(msg) => frame.render_widget(
            Paragraph::new(msg).style(Style::default().fg(theme.text)),
            area,
        ),
    }
}

fn render_image(frame: &mut Frame, area: Rect, path: &str) -> Result<(), String> {
    let img = load_image(path)?;
    let mut protocol = picker().new_resize_protocol(img);
    frame.render_stateful_widget(StatefulImage::default(), area, &mut protocol);
    Ok(())
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
}
