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
use splashboard::theme::Theme;

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
                // Skip renderers that animate. A single-frame PNG-equivalent catches the
                // effect at process start (e.g. `fade_in` is near-black at t=0), which reads
                // as "broken snapshot" rather than "preview of the motion". The renderer's
                // reference page still documents it; there's just no static preview.
                .filter(|r| !r.animates())
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
        "text_plain" | "animated_typewriter" => (40, 2),
        // Half-block glyphs need ~4 cols per char (Quadrant mode); widen the canvas so samples
        // up to ~20 chars render without clipping, and keep a row of headroom below the glyphs.
        "text_ascii" => (80, 5),
        "list_plain" => (40, 4),
        "status_badge" => (40, 1),
        "grid_table" => (40, 5),
        "gauge_circle" => (40, 3),
        "gauge_line" => (40, 1),
        "chart_sparkline" => (40, 3),
        "chart_line" | "chart_scatter" => (50, 10),
        "chart_bar" => (40, 8),
        "chart_pie" => (40, 10),
        "media_image" => (40, 5),
        "grid_calendar" => (28, 10),
        "grid_heatmap" => (40, 6),
        "list_timeline" => (50, 8),
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
    let theme = Theme::default();
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 0, width, height);
            render_payload(frame, area, &payload, spec, registry, &theme);
        })
        .expect("draw");
    buffer_to_html(terminal.backend().buffer())
}
