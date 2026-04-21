use ratatui::{Frame, layout::Rect};
use tui_big_text::{BigText, PixelSize};

use crate::payload::BignumData;

pub fn render(frame: &mut Frame, area: Rect, data: &BignumData) {
    let big = BigText::builder()
        .pixel_size(PixelSize::Full)
        .lines(vec![data.text.clone().into()])
        .build();
    frame.render_widget(big, area);
}

#[cfg(test)]
mod tests {
    use crate::payload::{BignumData, Body, Payload};
    use crate::render::test_utils::render_to_buffer;

    fn payload(text: &str) -> Payload {
        Payload {
            title: None,
            icon: None,
            status: None,
            format: None,
            body: Body::Bignum(BignumData { text: text.into() }),
        }
    }

    #[test]
    fn renders_without_panicking() {
        let _ = render_to_buffer(&payload("12:34"), 40, 10);
    }
}
