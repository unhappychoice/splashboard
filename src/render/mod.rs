use ratatui::{Frame, layout::Rect};

use crate::payload::{Body, Payload};

mod bar_chart;
mod bignum;
mod gauge;
mod image;
mod line_chart;
mod list;
mod sparkline;
mod text;

#[cfg(test)]
pub mod test_utils;

pub fn render_payload(frame: &mut Frame, area: Rect, payload: &Payload) {
    match &payload.body {
        Body::Text(d) => text::render(frame, area, d),
        Body::List(d) => list::render(frame, area, d),
        Body::Gauge(d) => gauge::render(frame, area, d),
        Body::Sparkline(d) => sparkline::render(frame, area, d),
        Body::LineChart(d) => line_chart::render(frame, area, d),
        Body::BarChart(d) => bar_chart::render(frame, area, d),
        Body::Bignum(d) => bignum::render(frame, area, d),
        Body::Image(d) => image::render(frame, area, d),
    }
}
