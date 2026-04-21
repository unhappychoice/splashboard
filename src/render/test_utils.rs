use std::collections::HashMap;

use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

use crate::layout::{self, Layout, WidgetId};
use crate::payload::Payload;
use crate::render::render_payload;

pub fn render_to_buffer(payload: &Payload, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_payload(f, f.area(), payload))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub fn render_to_buffer_with(
    root: &Layout,
    widgets: &HashMap<WidgetId, Payload>,
    width: u16,
    height: u16,
) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| layout::draw(f, f.area(), root, widgets))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub fn line_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
        .collect()
}
