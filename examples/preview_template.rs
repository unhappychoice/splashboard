//! Preview a bundled template with sample data.
//!
//! ```
//! cargo run --example preview_template -- home_splash
//! cargo run --example preview_template -- home_daily 120 40
//! ```
//!
//! Renders the named template into a headless `TestBackend` at the requested size
//! (default 100×40), using each widget's `sample_body` as the payload, then prints
//! the resulting cell grid to stdout as plain text. Useful for shape / layout
//! sanity checks; for full-fidelity output (figlet colouring, animated_postfx)
//! run `splashboard` itself against a throwaway `SPLASHBOARD_HOME`.

use std::collections::HashMap;

use ratatui::{Terminal, backend::TestBackend};

use splashboard::config::{Config, DashboardConfig, SettingsConfig, WidgetConfig};
use splashboard::fetcher::{FetchContext, RegisteredFetcher, Registry as FetcherRegistry};
use splashboard::layout::{self, WidgetId};
use splashboard::payload::{Body, ImageData, Payload, TextBlockData, TextData};
use splashboard::render::{Registry as RenderRegistry, RenderSpec, Shape};
use splashboard::runtime;
use splashboard::samples;
use splashboard::templates::{self, Template};
use splashboard::theme::Theme;

fn main() {
    let mut args = std::env::args().skip(1);
    let requested = args.next();
    let width: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let height: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(40);

    let Some(name) = requested.as_deref() else {
        list_templates();
        return;
    };
    let Some(template) = templates::find(name) else {
        eprintln!("unknown template: {name}\n");
        list_templates();
        std::process::exit(1);
    };
    render_template(template, width, height);
}

fn list_templates() {
    println!("Available templates:");
    for t in templates::TEMPLATES {
        println!("  {:<28} {:?}  {}", t.name, t.context, t.description);
    }
    println!("\nUsage: cargo run --example preview_template -- <name> [width] [height]");
}

fn render_template(template: &Template, width: u16, height: u16) {
    let dashboard =
        DashboardConfig::parse(template.body).expect("template TOML must parse (checked in tests)");
    let config = Config::from_parts(SettingsConfig::default(), dashboard);
    let fetchers = FetcherRegistry::with_builtins();
    let renderers = RenderRegistry::with_builtins();
    let shapes = runtime::derive_shapes(&config.widgets, &fetchers, &renderers);

    let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
    let mut specs: HashMap<WidgetId, RenderSpec> = HashMap::new();
    for w in &config.widgets {
        let shape = shapes
            .get(&w.id)
            .copied()
            .or_else(|| fetchers.get(&w.fetcher).map(|f| f.default_shape()))
            .unwrap_or(Shape::Text);
        let fetcher = fetchers
            .get(&w.fetcher)
            .unwrap_or_else(|| panic!("unknown fetcher {}", w.fetcher));
        payloads.insert(w.id.clone(), preview_payload(w, &fetcher, shape));
        if let Some(spec) = w.render.clone() {
            specs.insert(w.id.clone(), spec);
        }
    }

    let layout = config.to_layout();
    let theme = Theme::default();
    let loading = HashMap::new();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    // `animated_postfx` keys its effect off `Instant::now()` elapsed since the first call
    // to its internal `process_start()`. Draw once so that static `OnceLock` latches at
    // "now", sleep past the longest default duration (1500ms in the templates), then
    // draw again — the second pass sees the final, rest-state frame.
    let mut draw_once = || {
        terminal
            .draw(|f| {
                layout::draw(
                    f,
                    f.area(),
                    &layout,
                    &payloads,
                    &specs,
                    &renderers,
                    &theme,
                    &config.general,
                    &loading,
                );
            })
            .unwrap();
    };
    draw_once();
    std::thread::sleep(std::time::Duration::from_millis(1600));
    draw_once();
    let buf = terminal.backend().buffer().clone();

    let bar = "─".repeat(width as usize);
    println!("{bar}");
    println!("{}  ({}×{})", template.name, width, height);
    println!("{bar}");
    for y in 0..buf.area.height {
        let line: String = (0..buf.area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect();
        println!("{}", line.trim_end());
    }
    println!("{bar}");
}

/// Pick a stand-in payload for the preview. Realtime fetchers (clock / system / clock_*)
/// run `compute()` directly so the widget's configured `format` / `options` — the pieces
/// `sample_body` can't see — actually affect the output. Static widgets synthesise their
/// format. Everything else falls back to `sample_body` (async fetchers can't run inline
/// without a tokio runtime; their samples are close enough for a layout preview).
fn preview_payload(w: &WidgetConfig, fetcher: &RegisteredFetcher, shape: Shape) -> Payload {
    if w.fetcher == "static" {
        return wrap(static_body(w.format.as_deref().unwrap_or(""), shape));
    }
    if let Some(rt) = fetcher.as_realtime() {
        let ctx = FetchContext {
            widget_id: w.id.clone(),
            format: w.format.clone(),
            shape: Some(shape),
            options: w.options.clone(),
            ..Default::default()
        };
        return rt.compute(&ctx);
    }
    wrap(
        fetcher
            .sample_body(shape)
            .or_else(|| samples::canonical_sample(shape))
            .unwrap_or_else(|| {
                Body::Image(ImageData {
                    path: "/tmp/splashboard-preview-nonexistent.png".into(),
                })
            }),
    )
}

fn wrap(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

fn static_body(format: &str, shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: format.to_string(),
        }),
        _ => {
            let lines = if format.is_empty() {
                Vec::new()
            } else {
                format.split('\n').map(String::from).collect()
            };
            Body::TextBlock(TextBlockData { lines })
        }
    }
}
