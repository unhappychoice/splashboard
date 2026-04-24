//! Sample-driven preview rendering for the install-time gallery picker.
//!
//! Builds a `Payload` per widget using each fetcher's `sample_body` (or the shape-level
//! canonical sample as a fallback), then runs the real layout engine with those samples.
//! Same technique the docs-site snapshot generator uses — zero network / filesystem
//! access, deterministic output, matches what the user will actually see after install.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::config::{Config, DashboardConfig, SettingsConfig, WidgetConfig};
use crate::fetcher::{RegisteredFetcher, Registry as FetcherRegistry};
use crate::layout as layout_engine;
use crate::layout::WidgetId;
use crate::payload::{Body, ImageData, Payload, TextBlockData, TextData};
use crate::render::{
    Registry as RenderRegistry, RenderOptions, RenderSpec, Shape, default_renderer_for,
};
use crate::samples;
use crate::theme::Theme;

pub(crate) struct TemplatePreview {
    config: Config,
    payloads: HashMap<WidgetId, Payload>,
    specs: HashMap<WidgetId, RenderSpec>,
    theme: Theme,
    renderers: RenderRegistry,
}

impl TemplatePreview {
    /// Parses the template body and pre-builds every widget payload. Returns `None` for
    /// templates that fail to parse — callers render a fallback "preview unavailable"
    /// string rather than crashing the picker.
    pub(crate) fn from_body(body: &str) -> Option<Self> {
        Self::from_body_with_theme(body, Theme::default())
    }

    /// Same as [`from_body`] but paints the preview under an arbitrary theme — used by
    /// the theme picker so each candidate shows what the dashboard would look like.
    pub(crate) fn from_body_with_theme(body: &str, theme: Theme) -> Option<Self> {
        let dashboard = DashboardConfig::parse(body).ok()?;
        let config = Config::from_parts(SettingsConfig::default(), dashboard);
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let payloads = build_payloads(&config.widgets, &fetchers, &renderers);
        let specs = build_specs(&config.widgets);
        Some(Self {
            config,
            payloads,
            specs,
            theme,
            renderers,
        })
    }

    pub(crate) fn draw(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        frame.render_widget(
            Block::default().style(Style::default().bg(self.theme.bg)),
            area,
        );
        let root = self.config.to_layout();
        let loading: HashMap<WidgetId, Shape> = HashMap::new();
        layout_engine::draw(
            frame,
            area,
            &root,
            &self.payloads,
            &self.specs,
            &self.renderers,
            &self.theme,
            &loading,
        );
    }
}

fn build_payloads(
    widgets: &[WidgetConfig],
    fetchers: &FetcherRegistry,
    renderers: &RenderRegistry,
) -> HashMap<WidgetId, Payload> {
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = fetchers.get(&w.fetcher)?;
            let shape = resolve_shape(w, &fetcher, renderers);
            let body = override_from_format(w, shape)
                .or_else(|| fetcher.sample_body(shape))
                .or_else(|| samples::canonical_sample(shape))
                .unwrap_or_else(|| {
                    Body::Image(ImageData {
                        path: "/dev/null".into(),
                    })
                });
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
) -> Shape {
    let shapes = fetcher.shapes();
    let accepted = w
        .render
        .as_ref()
        .and_then(|spec| renderers.get(spec.renderer_name()))
        .map(|r| r.accepts().to_vec());
    if let Some(accepted) = accepted {
        if let Some(shape) = shapes.iter().find(|s| accepted.contains(s)) {
            return *shape;
        }
        if let Some(&first) = accepted.first() {
            return first;
        }
    }
    fetcher.default_shape()
}

/// `basic_static` carries its literal through `format = "..."`; honour it so the preview
/// reflects what the user would see. Other fetchers don't piggy-back on `format` for
/// sample output — this is a no-op for them.
fn override_from_format(w: &WidgetConfig, shape: Shape) -> Option<Body> {
    if w.fetcher != "basic_static" {
        return None;
    }
    let fmt = w.format.as_ref()?;
    match shape {
        Shape::Text => Some(Body::Text(TextData { value: fmt.clone() })),
        Shape::TextBlock => Some(Body::TextBlock(TextBlockData {
            lines: fmt.lines().map(String::from).collect(),
        })),
        _ => None,
    }
}

fn build_specs(widgets: &[WidgetConfig]) -> HashMap<WidgetId, RenderSpec> {
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

/// Animations look frozen at `t=0` (fade_in at 0% opacity, sweep_in at 0 progress) so the
/// preview would show a near-blank panel for anything wrapped in `animated_postfx` /
/// `animated_typewriter`. Swap those out for their underlying renderer's resting state so
/// the picker shows the final look of the widget.
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
        RenderSpec::Short(ref name) if name == "animated_boot" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_boot" => RenderSpec::Full {
            type_name: options.inner.clone().unwrap_or_else(|| "text_plain".into()),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_scanlines" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_scanlines" => RenderSpec::Full {
            type_name: options.inner.clone().unwrap_or_else(|| "text_plain".into()),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_splitflap" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_splitflap" => RenderSpec::Full {
            type_name: options.inner.clone().unwrap_or_else(|| "text_plain".into()),
            options: options.clone(),
        },
        RenderSpec::Short(ref name) if name == "animated_wave" => {
            RenderSpec::Short("text_plain".into())
        }
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_wave" => RenderSpec::Full {
            type_name: options.inner.clone().unwrap_or_else(|| "text_plain".into()),
            options: options.clone(),
        },
        // animated_figlet_morph's resting frame = text_ascii with the sequence's last font.
        // No inner knob here — the renderer always delegates to text_ascii — so swap in a
        // matching text_ascii spec for the preview.
        RenderSpec::Short(ref name) if name == "animated_figlet_morph" => RenderSpec::Full {
            type_name: "text_ascii".into(),
            options: RenderOptions {
                style: Some("figlet".into()),
                font: Some("ansi_shadow".into()),
                ..RenderOptions::default()
            },
        },
        RenderSpec::Full {
            ref type_name,
            ref options,
        } if type_name == "animated_figlet_morph" => {
            let resting_font = options
                .font_sequence
                .as_ref()
                .and_then(|seq| seq.last().cloned())
                .unwrap_or_else(|| "ansi_shadow".into());
            RenderSpec::Full {
                type_name: "text_ascii".into(),
                options: RenderOptions {
                    style: Some("figlet".into()),
                    font: Some(resting_font),
                    color: options.color.clone(),
                    align: options.align.clone(),
                    ..RenderOptions::default()
                },
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn every_template_builds_a_preview() {
        for t in templates::TEMPLATES {
            assert!(
                TemplatePreview::from_body(t.body).is_some(),
                "{} failed to build preview",
                t.name
            );
        }
    }

    #[test]
    fn draw_does_not_panic_on_small_area() {
        let preview = TemplatePreview::from_body(templates::TEMPLATES[0].body).unwrap();
        let backend = TestBackend::new(40, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| preview.draw(frame, frame.area()))
            .unwrap();
    }

    /// Theme picker renders the same template under each built-in preset. Guards against
    /// a preset-specific panic sneaking into the render path.
    #[test]
    fn every_theme_preset_renders_home_splash() {
        let body = templates::find("home_splash").unwrap().body;
        for name in crate::theme::presets::KNOWN.iter().copied() {
            let theme = crate::theme::presets::by_name(name).unwrap();
            let preview =
                TemplatePreview::from_body_with_theme(body, theme).expect("preview builds");
            let backend = TestBackend::new(100, 32);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| preview.draw(frame, frame.area()))
                .unwrap_or_else(|e| panic!("{name} failed to render: {e}"));
        }
    }
}
