//! Renders inline previews for each fetcher's page.
//!
//! The fetcher decides what a sample payload looks like — via the
//! [`RegisteredFetcher::sample_body`] trait method — and this module only handles canvas sizing
//! and the ratatui → HTML conversion. Adding a new fetcher doesn't touch this file; editing a
//! sample means editing the fetcher itself.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use splashboard::fetcher::RegisteredFetcher;
use splashboard::payload::{Body, Payload};
use splashboard::render::{Registry as RenderRegistry, RenderSpec, Shape, render_payload};

use crate::html_snapshot::buffer_to_html;

pub struct Preview {
    pub shape: Shape,
    pub renderer: String,
    pub html: String,
    pub config_snippet: String,
}

pub fn previews_for(fetcher: &RegisteredFetcher, renderers: &RenderRegistry) -> Vec<Preview> {
    let sorted = renderers.sorted();
    fetcher
        .shapes()
        .iter()
        .flat_map(|shape| fetcher.sample_body(*shape).map(|body| (shape, body)))
        .flat_map(|(shape, body)| {
            sorted
                .iter()
                .filter(|r| r.accepts().contains(shape))
                .map(|r| {
                    let (w, h) = renderer_dimensions(r.name());
                    let spec = RenderSpec::Short(r.name().into());
                    let html = render_html(w, h, &body, Some(&spec), renderers);
                    Preview {
                        shape: *shape,
                        renderer: r.name().to_string(),
                        html,
                        config_snippet: config_snippet(fetcher.name(), r.name()),
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Per-renderer canvas size. Picked so each renderer has enough room to show its characteristic
/// output without overflowing a typical docs column.
fn renderer_dimensions(renderer: &str) -> (u16, u16) {
    match renderer {
        "simple" | "animated_typewriter" => (40, 2),
        // Half-block glyphs need ~4 cols per char (Quadrant mode); widen the canvas so samples
        // up to ~20 chars render without clipping, and keep a row of headroom below the glyphs.
        "ascii_art" => (80, 5),
        "list" => (40, 4),
        "badge" => (40, 1),
        "table" => (40, 5),
        "gauge" => (40, 3),
        "line_gauge" => (40, 1),
        "sparkline" => (40, 3),
        "chart_line" | "scatter" => (50, 10),
        "bar_chart" => (40, 8),
        "pie_chart" => (40, 10),
        "image" => (40, 5),
        "calendar" => (28, 10),
        "heatmap" => (40, 6),
        "timeline" => (50, 8),
        _ => (40, 5),
    }
}

fn config_snippet(fetcher: &str, renderer: &str) -> String {
    format!("[[widget]]\nid = \"{fetcher}\"\nfetcher = \"{fetcher}\"\nrender = \"{renderer}\"")
}

fn render_html(
    width: u16,
    height: u16,
    body: &Body,
    spec: Option<&RenderSpec>,
    registry: &RenderRegistry,
) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    let payload = Payload {
        icon: None,
        status: None,
        format: None,
        body: body.clone(),
    };
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 0, width, height);
            render_payload(frame, area, &payload, spec, registry);
        })
        .expect("draw");
    buffer_to_html(terminal.backend().buffer())
}
