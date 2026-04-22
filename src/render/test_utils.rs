use std::collections::HashMap;

use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

use crate::layout::{self, Layout, WidgetId};
use crate::payload::Payload;
use crate::render::{Registry, RenderSpec, render_payload};

/// Render a single payload through the default renderer for its shape.
pub fn render_to_buffer(payload: &Payload, width: u16, height: u16) -> Buffer {
    render_to_buffer_with_spec(payload, None, &Registry::with_builtins(), width, height)
}

/// Render a single payload through a specific renderer spec — used when a test needs to
/// exercise a non-default renderer (e.g. `ascii_art` for a `Text` payload).
pub fn render_to_buffer_with_spec(
    payload: &Payload,
    spec: Option<&RenderSpec>,
    registry: &Registry,
    width: u16,
    height: u16,
) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_payload(f, f.area(), payload, spec, registry))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Render a whole layout with its widget payloads. Render specs default per-shape; callers who
/// need custom renderers should use [`render_to_buffer_with_layout_and_specs`].
pub fn render_to_buffer_with(
    root: &Layout,
    widgets: &HashMap<WidgetId, Payload>,
    width: u16,
    height: u16,
) -> Buffer {
    render_to_buffer_with_layout_and_specs(root, widgets, &HashMap::new(), width, height)
}

pub fn render_to_buffer_with_layout_and_specs(
    root: &Layout,
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    width: u16,
    height: u16,
) -> Buffer {
    let registry = Registry::with_builtins();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| layout::draw(f, f.area(), root, widgets, specs, &registry))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub fn line_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
        .collect()
}
