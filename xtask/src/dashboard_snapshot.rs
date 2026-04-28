//! Render a whole dashboard TOML to an inline HTML `<pre>` without running any fetchers.
//!
//! Uses each fetcher's `sample_body()` (the same source of truth the per-widget reference
//! previews use) composed through the real layout engine. Result: a docs-site surface that
//! shows exactly what the splash looks like when you adopt a config, with zero network /
//! filesystem access at build time.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use splashboard::config::{Config, DashboardConfig, SettingsConfig, WidgetConfig};
use splashboard::fetcher::{FetchContext, RegisteredFetcher, Registry as FetcherRegistry};
use splashboard::layout as layout_engine;
use splashboard::payload::{Body, Payload, TextBlockData, TextData};
use splashboard::render::{
    Registry as RenderRegistry, RenderOptions, RenderSpec, Shape, default_renderer_for,
};
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
                frame,
                area,
                &root,
                &payloads,
                &specs,
                &renderers,
                &theme,
                &config.general,
                &loading,
            );
        })
        .context("draw dashboard")?;
    Ok(buffer_to_html(terminal.backend().buffer()))
}

/// Resolve a `Payload` per widget. Realtime fetchers (`clock`, `clock_derived`, `system`, …)
/// compute live through their real pipeline so format / options land in the output; cached
/// fetchers fall back to `sample_body` since running them would need network / disk access.
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
            if let Some(realtime) = fetcher.as_realtime() {
                let ctx = FetchContext {
                    widget_id: w.id.clone(),
                    format: w.format.clone(),
                    timeout: Duration::from_millis(50),
                    file_format: None,
                    shape: Some(shape),
                    options: w.options.clone(),
                    timezone: None,
                    locale: None,
                };
                return Some((w.id.clone(), realtime.compute(&ctx)));
            }
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
            type_name: inner_renderer_name(options),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_boot" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_boot" => RenderSpec::Full {
            type_name: inner_renderer_name(options),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_scanlines" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_scanlines" => RenderSpec::Full {
            type_name: inner_renderer_name(options),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_splitflap" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_splitflap" => RenderSpec::Full {
            type_name: inner_renderer_name(options),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_wave" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_wave" => RenderSpec::Full {
            type_name: inner_renderer_name(options),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_figlet_morph" => RenderSpec::Full {
            type_name: "text_ascii".into(),
            options: RenderOptions {
                style: Some("figlet".into()),
                ..RenderOptions::default()
            }
            .with_extra("font", "ansi_shadow"),
        },
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_figlet_morph" => {
            let resting_font = font_sequence(options)
                .and_then(|seq| seq.last().cloned())
                .unwrap_or_else(|| "ansi_shadow".into());
            RenderSpec::Full {
                type_name: "text_ascii".into(),
                options: RenderOptions {
                    style: Some("figlet".into()),
                    color: options.color.clone(),
                    align: options.align.clone(),
                    ..RenderOptions::default()
                }
                .with_extra("font", resting_font),
            }
        }
        other => other,
    }
}

fn inner_renderer_name(opts: &RenderOptions) -> String {
    opts.extra_str("inner")
        .map(String::from)
        .unwrap_or_else(|| "text_plain".into())
}

fn font_sequence(opts: &RenderOptions) -> Option<Vec<String>> {
    opts.extra_string_array("font_sequence")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn widget(id: &str, fetcher: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: fetcher.into(),
            render: None,
            format: None,
            refresh_interval: None,
            file_format: None,
            options: None,
        }
    }

    fn write_dashboard(body: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "splashboard-dashboard-snapshot-{unique}-{}.toml",
            std::process::id()
        ));
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn render_config_html_renders_basic_static_dashboard() {
        let path = write_dashboard(
            r#"
[[widget]]
id = "hello"
fetcher = "basic_static"
format = "Hi"
render = "list_plain"

[[row]]
height = { length = 1 }
  [[row.child]]
  widget = "hello"
"#,
        );
        let html = render_config_html(&path, 12, 4).unwrap();
        fs::remove_file(path).unwrap();

        assert!(html.starts_with("<pre class=\"splash-snapshot\">"));
        assert!(html.contains("<span class=\"c\">H</span>"));
        assert!(html.contains("<span class=\"c\">i</span>"));
        assert!(html.ends_with("</pre>"));
    }

    #[test]
    fn sample_payloads_use_realtime_compute_and_basic_static_override() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();

        let mut static_widget = widget("static", "basic_static");
        static_widget.format = Some("alpha\nbeta".into());
        static_widget.render = Some(RenderSpec::Short("list_plain".into()));

        let mut clock_widget = widget("clock", "clock");
        clock_widget.render = Some(RenderSpec::Short("grid_table".into()));

        let payloads = sample_payloads(&[static_widget, clock_widget], &fetchers, &renderers);

        assert!(matches!(
            &payloads.get("static").unwrap().body,
            Body::TextBlock(TextBlockData { lines })
                if lines == &vec![String::from("alpha"), String::from("beta")]
        ));
        assert!(matches!(
            payloads.get("clock").map(|payload| &payload.body),
            Some(Body::Entries(_))
        ));
    }

    #[test]
    fn resolve_shape_prefers_shared_shape_and_falls_back_to_renderer_shape() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let fetcher = fetchers.get("basic_static").unwrap();

        let mut shared = widget("shared", "basic_static");
        shared.render = Some(RenderSpec::Short("list_plain".into()));
        assert_eq!(
            resolve_shape(&shared, &fetcher, &renderers),
            Some(Shape::TextBlock)
        );

        let mut mismatch = widget("mismatch", "basic_static");
        mismatch.render = Some(RenderSpec::Short("status_badge".into()));
        assert_eq!(
            resolve_shape(&mismatch, &fetcher, &renderers),
            Some(Shape::Badge)
        );
    }

    #[test]
    fn widget_specs_default_and_deanimate_wrapped_renderers() {
        let plain = widget("plain", "basic_static");

        let mut typewriter = widget("typewriter", "basic_static");
        typewriter.render = Some(RenderSpec::Short("animated_typewriter".into()));

        let mut postfx = widget("postfx", "basic_static");
        postfx.render = Some(RenderSpec::Full {
            type_name: "animated_postfx".into(),
            options: RenderOptions {
                align: Some("center".into()),
                ..RenderOptions::default()
            }
            .with_extra("inner", "list_plain"),
        });

        let specs = widget_specs(&[plain, typewriter, postfx]);

        assert!(matches!(
            specs.get("plain"),
            Some(RenderSpec::Short(name)) if name == "text_plain"
        ));
        assert!(matches!(
            specs.get("typewriter"),
            Some(RenderSpec::Short(name)) if name == "text_plain"
        ));
        assert!(matches!(
            specs.get("postfx"),
            Some(RenderSpec::Full { type_name, options })
                if type_name == "list_plain" && options.align.as_deref() == Some("center")
        ));
    }

    #[test]
    fn deanimate_figlet_morph_uses_last_font_and_preserves_style_options() {
        let spec = RenderSpec::Full {
            type_name: "animated_figlet_morph".into(),
            options: RenderOptions {
                align: Some("right".into()),
                color: Some("panel_title".into()),
                ..RenderOptions::default()
            }
            .with_extra("font_sequence", vec!["small", "doom"]),
        };

        let deanimated = deanimate(spec);

        assert!(matches!(
            deanimated,
            RenderSpec::Full { type_name, options }
                if type_name == "text_ascii"
                    && options.style.as_deref() == Some("figlet")
                    && options.align.as_deref() == Some("right")
                    && options.color.as_deref() == Some("panel_title")
                    && options.extra_str("font") == Some("doom")
        ));
    }
}
