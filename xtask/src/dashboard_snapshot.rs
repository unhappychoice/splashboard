//! Render a whole dashboard TOML to an inline HTML `<pre>` without running any fetchers.
//!
//! Uses each fetcher's `sample_body()` (the same source of truth the per-widget reference
//! previews use) composed through the real layout engine. Result: a docs-site surface that
//! shows exactly what the splash looks like when you adopt a config, with zero network /
//! filesystem access at build time.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use splashboard::config::{Config, DashboardConfig, SettingsConfig, WidgetConfig};
use splashboard::fetcher::{RegisteredFetcher, Registry as FetcherRegistry};
use splashboard::layout as layout_engine;
use splashboard::payload::{Body, Payload, TextBlockData, TextData};
use splashboard::render::{Registry as RenderRegistry, RenderSpec, Shape, default_renderer_for};
use splashboard::theme::Theme;

use crate::html_snapshot::buffer_to_html;

pub fn render_config_html(config_path: &Path, width: u16, height: u16) -> Result<String> {
    let body = fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let dashboard = DashboardConfig::parse(&body)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", config_path.display()))?;
    let config = Config::from_parts(SettingsConfig::default(), dashboard);
    let fetchers = FetcherRegistry::with_builtins();
    let renderers = RenderRegistry::with_builtins();
    let theme = Theme::default();
    let payloads = sample_payloads(&config.widgets, &fetchers, &renderers);
    let specs = widget_specs(&config.widgets);
    let root = config.to_layout();

    // All widgets already have sample payloads → none are "loading". Passing an empty map
    // keeps the runtime's loading-placeholder branch disabled for the preview.
    let loading: HashMap<String, Shape> = HashMap::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).context("init offscreen terminal")?;
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 0, width, height);
            frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);
            layout_engine::draw(
                frame, area, &root, &payloads, &specs, &renderers, &theme, &loading,
            );
        })
        .context("draw dashboard")?;
    Ok(buffer_to_html(terminal.backend().buffer()))
}

/// Resolve a sample `Payload` per widget using the fetcher-preferred shape intersected with
/// the renderer's accepted shape — same rule the runtime applies.
fn sample_payloads(
    widgets: &[WidgetConfig],
    fetchers: &FetcherRegistry,
    renderers: &RenderRegistry,
) -> HashMap<String, Payload> {
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = fetchers.get(&w.fetcher)?;
            let shape = resolve_shape(w, &fetcher, renderers)?;
            let body = override_from_format(w, shape).or_else(|| fetcher.sample_body(shape))?;
            Some((
                w.id.clone(),
                Payload {
                    icon: None,
                    status: None,
                    format: w.format.clone(),
                    body,
                },
            ))
        })
        .collect()
}

fn resolve_shape(
    w: &WidgetConfig,
    fetcher: &RegisteredFetcher,
    renderers: &RenderRegistry,
) -> Option<Shape> {
    let shapes = fetcher.shapes();
    let accepted = w
        .render
        .as_ref()
        .and_then(|spec| renderers.get(spec.renderer_name()))
        .map(|r| r.accepts().to_vec());
    if let Some(accepted) = accepted {
        if let Some(shape) = shapes.iter().find(|s| accepted.contains(s)) {
            return Some(*shape);
        }
        if let Some(&first) = accepted.first() {
            return Some(first);
        }
    }
    Some(fetcher.default_shape())
}

/// `basic_static` lets each widget carry a literal via `format = "..."`; honour it so the
/// preview matches what the user would see. Other fetchers don't use widget.format for sample
/// output, so this is a no-op for them.
fn override_from_format(w: &WidgetConfig, shape: Shape) -> Option<Body> {
    if w.fetcher != "basic_static" {
        return None;
    }
    let fmt = w.format.as_ref()?;
    Some(match shape {
        Shape::Text => Body::Text(TextData { value: fmt.clone() }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: fmt.lines().map(String::from).collect(),
        }),
        _ => return None,
    })
}

fn widget_specs(widgets: &[WidgetConfig]) -> HashMap<String, RenderSpec> {
    widgets
        .iter()
        .map(|w| {
            let spec = w.render.clone().unwrap_or_else(|| {
                RenderSpec::Short(default_renderer_for(Shape::Text).to_string())
            });
            (w.id.clone(), deanimate(spec))
        })
        .collect()
}

/// Strip animated wrappers so the snapshot shows the effect's final state instead of
/// whatever happens to be on screen at process-start t=0 (which is near-invisible for
/// `fade_in` / `coalesce` / most other tachyonfx effects). The inner renderer keeps every
/// style option (font, color, align, …) that was carried on the outer spec.
///
/// - `animated_postfx` → its declared `inner` (default `text_plain` if absent).
/// - `animated_typewriter` → `text_plain` (the typewriter's final rested state).
fn deanimate(spec: RenderSpec) -> RenderSpec {
    match spec {
        RenderSpec::Short(ref name) if name == "animated_typewriter" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_typewriter" => RenderSpec::Full {
            type_name: "text_plain".into(),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_postfx" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_postfx" => RenderSpec::Full {
            type_name: options.inner.clone().unwrap_or_else(|| "text_plain".into()),
            options: options.clone(),
        },
        other => other,
    }
}
