//! Renders inline previews for each fetcher's and renderer's reference page.
//!
//! Fetcher pages drive previews from the fetcher's `sample_body` (one preview per
//! shape × compatible renderer). Renderer pages mirror that — one preview per accepted
//! shape, sourced from `samples::canonical_sample` so the example payload matches what
//! a fresh fetcher with no override would emit.

use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use splashboard::config::General;
use splashboard::fetcher::{RegisteredFetcher, Registry as FetcherRegistry};
use splashboard::payload::{Body, Payload};
use splashboard::render::{
    Registry as RenderRegistry, RenderSpec, Renderer, Shape, render_payload,
};
use splashboard::samples;
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
        "gauge_battery" => (40, 5),
        "gauge_circle" => (40, 3),
        "gauge_line" => (40, 1),
        "gauge_thermometer" => (20, 8),
        "chart_sparkline" => (40, 3),
        "chart_line" | "chart_scatter" => (50, 10),
        "chart_bar" => (40, 8),
        "chart_histogram" => (40, 6),
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

/// One preview per shape this renderer accepts. Mirrors [`previews_for`] from the renderer
/// side: instead of fixing a fetcher and walking compatible renderers, we fix the renderer
/// and source a representative payload per shape from [`samples::canonical_sample`].
///
/// Animated renderers are skipped — same rationale as the fetcher side, the t=0 frame reads
/// as a broken snapshot. The renderer page still documents the effect via prose.
///
/// Wrapper renderers (`animated_postfx` & friends) accept every shape; they're animated so
/// they fall under the same skip rule, which conveniently avoids emitting ~12 previews each.
pub fn previews_for_renderer(
    renderer: &Arc<dyn Renderer>,
    fetchers: &FetcherRegistry,
    renderers: &RenderRegistry,
) -> Vec<Preview> {
    if renderer.animates() {
        return Vec::new();
    }
    renderer
        .accepts()
        .iter()
        .flat_map(|shape| samples::canonical_sample(*shape).map(|body| (shape, body)))
        .map(|(shape, body)| {
            let (w, h) = renderer_dimensions(renderer.name());
            let spec = RenderSpec::Short(renderer.name().into());
            let html = render_html(w, h, &body, Some(&spec), renderers);
            let fetcher_name = representative_fetcher(*shape, fetchers);
            Preview {
                shape: *shape,
                renderer: renderer.name().to_string(),
                html,
                config_snippet: config_snippet(&fetcher_name, renderer.name()),
            }
        })
        .collect()
}

/// Pick a stable fetcher name to surface in the renderer-page config snippet. Per-shape
/// preference table keeps the example illustrative (`clock` for Text instead of whatever
/// sorts first); falls back to the first matching fetcher in the sorted registry, then to
/// `basic_read_store` as the universal escape hatch — that last branch is unreachable
/// today but future-proofs the table against new shapes.
fn representative_fetcher(shape: Shape, fetchers: &FetcherRegistry) -> String {
    let preferred = match shape {
        Shape::Text => "clock",
        Shape::TextBlock => "git_recent_commits",
        Shape::MarkdownTextBlock => "wikipedia_featured",
        Shape::LinkedTextBlock => "rss",
        Shape::Entries => "system",
        Shape::Ratio => "clock_ratio",
        Shape::NumberSeries => "git_commits_activity",
        Shape::PointSeries => "weather",
        Shape::Bars => "github_languages",
        Shape::Image => "github_avatar",
        Shape::Calendar => "clock",
        Shape::Heatmap => "git_blame_heatmap",
        Shape::Badge => "github_action_status",
        Shape::Timeline => "git_recent_commits",
        Shape::Error => "",
    };
    if !preferred.is_empty()
        && fetchers
            .get(preferred)
            .map(|f| f.shapes().contains(&shape))
            .unwrap_or(false)
    {
        return preferred.to_string();
    }
    fetchers
        .sorted()
        .iter()
        .find(|f| f.shapes().contains(&shape))
        .map(|f| f.name().to_string())
        .unwrap_or_else(|| "basic_read_store".into())
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
            render_payload(
                frame,
                area,
                &payload,
                spec,
                registry,
                &theme,
                &General::default(),
            );
        })
        .expect("draw");
    buffer_to_html(terminal.backend().buffer())
}
