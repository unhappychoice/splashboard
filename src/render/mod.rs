use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

use crate::payload::{Body, Payload};

mod bar_chart;
mod bignum;
mod gauge;
mod line_chart;
mod list;
mod sparkline;
mod text;

#[cfg(test)]
pub mod test_utils;

pub fn render_payload(frame: &mut Frame, area: Rect, payload: &Payload) {
    let block = build_block(payload);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match &payload.body {
        Body::Text(d) => text::render(frame, inner, d),
        Body::List(d) => list::render(frame, inner, d),
        Body::Gauge(d) => gauge::render(frame, inner, d),
        Body::Sparkline(d) => sparkline::render(frame, inner, d),
        Body::LineChart(d) => line_chart::render(frame, inner, d),
        Body::BarChart(d) => bar_chart::render(frame, inner, d),
        Body::Bignum(d) => bignum::render(frame, inner, d),
        Body::Image(_) => {
            frame.render_widget(Paragraph::new("[image: renderer pending]"), inner);
        }
    }
}

fn build_block(payload: &Payload) -> Block<'_> {
    let b = Block::default().borders(Borders::ALL);
    match payload.title.as_deref() {
        Some(title) => b.title(title),
        None => b,
    }
}
