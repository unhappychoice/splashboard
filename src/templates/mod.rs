//! Curated dashboard templates shipped with splashboard. Each entry is a ready-to-run
//! `DashboardConfig` — users pick one via `splashboard install --template <name>` (#58)
//! and drop it into their `$HOME/.splashboard/<home|project>.dashboard.toml` or a repo's
//! `./.splashboard/dashboard.toml`. Embedded via `include_str!` so no filesystem lookup
//! is needed at install time.

/// Where a template is intended to apply. Drives the install-time picker (#58): users
/// pair one `Home` template with one `Project` template so both contexts look deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateContext {
    /// Home ambient splash — rendered when CWD isn't a git repo root.
    Home,
    /// Per-project splash — rendered when CWD is a git repo root and no per-dir override.
    Project,
}

#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub name: &'static str,
    pub context: TemplateContext,
    /// One-line description used by the install-time picker and docs gallery.
    pub description: &'static str,
    /// Raw TOML body. Valid `DashboardConfig` — parse via `DashboardConfig::parse`.
    pub body: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "home_splash",
        context: TemplateContext::Home,
        description: "Terminal name as a giant figlet hero with a postfx reveal, plus ambient weather / moon / year-progress.",
        body: include_str!("home_splash.toml"),
    },
    Template {
        name: "home_daily",
        context: TemplateContext::Home,
        description: "Large time-of-day hero; weather / sunrise / moon on the left and year/month/week/day gauges on the right.",
        body: include_str!("home_daily.toml"),
    },
    Template {
        name: "home_github",
        context: TemplateContext::Home,
        description: "Avatar + handle + contribution heatmap, plus open PRs / review queue and recent activity timeline.",
        body: include_str!("home_github.toml"),
    },
    Template {
        name: "home_minimal",
        context: TemplateContext::Home,
        description: "Quiet three-line preset: a time-of-day greeting, one-line date, and the daily quote — no figlet, lots of whitespace.",
        body: include_str!("home_minimal.toml"),
    },
    Template {
        name: "project_splash",
        context: TemplateContext::Project,
        description: "Repo name as a giant figlet hero with a postfx reveal, slug / description / license underneath — nothing else. For repo-shipped configs.",
        body: include_str!("project_splash.toml"),
    },
    Template {
        name: "project_github_activity",
        context: TemplateContext::Project,
        description: "Dynamic repo hero + subtitle; branch state, commit / CI sparklines, open PRs, and recent releases.",
        body: include_str!("project_github_activity.toml"),
    },
    Template {
        name: "project_minimal",
        context: TemplateContext::Project,
        description: "Quiet two-line preset: repo name + latest git tag. Fully offline (no GitHub calls) — acknowledges the repo without taking over the screen.",
        body: include_str!("project_minimal.toml"),
    },
];

/// Lookup by name. Returns `None` for unknown templates so callers can surface a
/// helpful "did you mean …" instead of panicking.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Templates matching the given context, in declaration order — stable ordering keeps
/// the install-time picker deterministic.
pub fn for_context(context: TemplateContext) -> impl Iterator<Item = &'static Template> {
    TEMPLATES.iter().filter(move |t| t.context == context)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::config::DashboardConfig;
    use crate::config::SettingsConfig;
    use crate::fetcher::Registry as FetcherRegistry;
    use crate::layout::WidgetId;
    use crate::payload::{Body, ImageData, Payload};
    use crate::render::{
        self, Registry as RenderRegistry, Shape, test_utils::render_to_buffer_with_layout_and_specs,
    };
    use crate::runtime;
    use crate::samples;

    fn render_registry() -> RenderRegistry {
        RenderRegistry::with_builtins()
    }

    fn fetcher_registry() -> FetcherRegistry {
        FetcherRegistry::with_builtins()
    }

    /// Synthetic payload for the given widget, using the fetcher's own `sample_body` when
    /// available and falling back to a shape-level canonical sample. `Image` is the one
    /// shape whose canonical sample is `None` (no portable path); we hand the renderer a
    /// non-existent path so it falls through its own "file missing" placeholder without
    /// panicking.
    fn sample_payload(fetcher: &crate::fetcher::RegisteredFetcher, shape: Shape) -> Payload {
        let body = fetcher
            .sample_body(shape)
            .or_else(|| samples::canonical_sample(shape))
            .unwrap_or_else(|| {
                Body::Image(ImageData {
                    path: "/tmp/splashboard-nonexistent.png".into(),
                })
            });
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }

    fn dashboard_for(template: &Template) -> DashboardConfig {
        DashboardConfig::parse(template.body)
            .unwrap_or_else(|e| panic!("{} failed to parse: {e}", template.name))
    }

    #[test]
    fn every_template_parses() {
        for t in TEMPLATES {
            let _ = dashboard_for(t);
        }
    }

    #[test]
    fn every_widget_fetcher_is_registered() {
        let reg = fetcher_registry();
        for t in TEMPLATES {
            let d = dashboard_for(t);
            for w in &d.widgets {
                assert!(
                    reg.get(&w.fetcher).is_some(),
                    "{}: widget `{}` uses unknown fetcher `{}`",
                    t.name,
                    w.id,
                    w.fetcher,
                );
            }
        }
    }

    #[test]
    fn every_widget_renderer_is_registered() {
        let reg = render_registry();
        for t in TEMPLATES {
            let d = dashboard_for(t);
            for w in &d.widgets {
                if let Some(spec) = &w.render {
                    let name = spec.renderer_name();
                    assert!(
                        reg.get(name).is_some(),
                        "{}: widget `{}` uses unknown renderer `{}`",
                        t.name,
                        w.id,
                        name,
                    );
                }
            }
        }
    }

    #[test]
    fn every_widget_id_is_referenced_by_layout() {
        for t in TEMPLATES {
            let d = dashboard_for(t);
            let defined: std::collections::HashSet<_> =
                d.widgets.iter().map(|w| w.id.clone()).collect();
            for row in &d.rows {
                for child in &row.children {
                    assert!(
                        defined.contains(&child.widget),
                        "{}: layout references unknown widget `{}`",
                        t.name,
                        child.widget,
                    );
                }
            }
        }
    }

    #[test]
    fn every_template_renders_without_panic() {
        let fetchers = fetcher_registry();
        let renderers = render_registry();
        for t in TEMPLATES {
            let d = dashboard_for(t);
            let config = crate::config::Config::from_parts(SettingsConfig::default(), d);
            let shapes = runtime::derive_shapes(&config.widgets, &fetchers, &renderers);

            let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
            let mut specs: HashMap<WidgetId, render::RenderSpec> = HashMap::new();
            for w in &config.widgets {
                let shape = shapes
                    .get(&w.id)
                    .copied()
                    .or_else(|| fetchers.get(&w.fetcher).map(|f| f.default_shape()))
                    .unwrap_or(Shape::Text);
                let fetcher = fetchers
                    .get(&w.fetcher)
                    .unwrap_or_else(|| panic!("{}: unknown fetcher `{}`", t.name, w.fetcher));
                payloads.insert(w.id.clone(), sample_payload(&fetcher, shape));
                if let Some(spec) = w.render.clone() {
                    specs.insert(w.id.clone(), spec);
                }
            }

            let layout = config.to_layout();
            let _ = render_to_buffer_with_layout_and_specs(&layout, &payloads, &specs, 100, 40);
        }
    }
}
