use std::collections::HashMap;
use std::sync::Arc;

use ratatui::{Frame, layout::Rect, widgets::Paragraph};
use serde::Deserialize;

use crate::payload::{Body, Payload};

mod animated_typewriter;
mod ascii_art;
mod bar_chart;
mod calendar;
mod gauge;
mod image;
mod line_chart;
mod line_gauge;
mod list;
mod scatter;
mod sparkline;
mod table;
mod text;

#[cfg(test)]
pub mod test_utils;

/// Data shape a fetcher produces. A shape is consumed by one or more renderers — a `Lines`
/// shape can be rendered by `simple` (plain text), `ascii_art` (big-text via tui-big-text),
/// and future alternatives (figlet, animated typewriter) without the fetcher needing to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Lines,
    Entries,
    Ratio,
    NumberSeries,
    PointSeries,
    Bars,
    Image,
    Calendar,
}

pub fn shape_of(body: &Body) -> Shape {
    match body {
        Body::Lines(_) => Shape::Lines,
        Body::Entries(_) => Shape::Entries,
        Body::Ratio(_) => Shape::Ratio,
        Body::NumberSeries(_) => Shape::NumberSeries,
        Body::PointSeries(_) => Shape::PointSeries,
        Body::Bars(_) => Shape::Bars,
        Body::Image(_) => Shape::Image,
        Body::Calendar(_) => Shape::Calendar,
    }
}

/// Per-renderer configuration. Each renderer picks out the fields it cares about from this bag;
/// others are ignored. Kept deliberately flat so config authors can write
/// `render = { type = "ascii_art", style = "figlet" }` without nesting.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderOptions {
    /// ascii_art: "blocks" (default, tui-big-text) | "figlet" (figlet-rs).
    #[serde(default)]
    pub style: Option<String>,
    /// ascii_art blocks style: "full" | "quadrant" | "sextant". None = adaptive (pick by area
    /// height). Ignored by other styles.
    #[serde(default)]
    pub pixel_size: Option<String>,
}

/// What the TOML accepts for `render`. Short form `render = "simple"` uses defaults; long form
/// `render = { type = "ascii_art", pixel_size = "quadrant" }` carries options. Absence means
/// "use the default renderer for this shape".
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RenderSpec {
    Short(String),
    Full {
        #[serde(rename = "type")]
        type_name: String,
        #[serde(flatten)]
        options: RenderOptions,
    },
}

impl RenderSpec {
    pub fn renderer_name(&self) -> &str {
        match self {
            RenderSpec::Short(s) => s,
            RenderSpec::Full { type_name, .. } => type_name,
        }
    }

    pub fn options(&self) -> RenderOptions {
        match self {
            RenderSpec::Short(_) => RenderOptions::default(),
            RenderSpec::Full { options, .. } => options.clone(),
        }
    }
}

pub trait Renderer: Send + Sync {
    fn name(&self) -> &str;
    fn accepts(&self) -> &[Shape];
    /// Whether this renderer produces different output on repeated calls within one draw
    /// cycle. The runtime consults this: any true answer upgrades the draw phase from a single
    /// one-shot call into a time-boxed multi-frame loop so the animation actually plays. Load
    /// state (cache warm / cold / --wait) is not consulted — load and render are independent
    /// axes.
    fn animates(&self) -> bool {
        false
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, opts: &RenderOptions);
}

#[derive(Default, Clone)]
pub struct Registry {
    renderers: HashMap<String, Arc<dyn Renderer>>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Arc::new(text::SimpleRenderer));
        r.register(Arc::new(ascii_art::AsciiArtRenderer));
        r.register(Arc::new(animated_typewriter::AnimatedTypewriterRenderer));
        r.register(Arc::new(list::ListRenderer));
        r.register(Arc::new(table::TableRenderer));
        r.register(Arc::new(gauge::GaugeRenderer));
        r.register(Arc::new(line_gauge::LineGaugeRenderer));
        r.register(Arc::new(sparkline::SparklineRenderer));
        r.register(Arc::new(line_chart::LineChartRenderer));
        r.register(Arc::new(scatter::ScatterRenderer));
        r.register(Arc::new(bar_chart::BarChartRenderer));
        r.register(Arc::new(image::ImageRenderer));
        r.register(Arc::new(calendar::CalendarRenderer));
        r
    }

    pub fn register(&mut self, r: Arc<dyn Renderer>) {
        self.renderers.insert(r.name().to_string(), r);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Renderer>> {
        self.renderers.get(name).cloned()
    }
}

pub fn default_renderer_for(shape: Shape) -> &'static str {
    match shape {
        Shape::Lines => "simple",
        Shape::Entries => "table",
        Shape::Ratio => "gauge",
        Shape::NumberSeries => "sparkline",
        Shape::PointSeries => "chart_line",
        Shape::Bars => "bar_chart",
        Shape::Image => "image",
        Shape::Calendar => "calendar",
    }
}

/// Resolves a renderer by spec (or shape default), compat-checks it, and renders. An unknown
/// renderer name or a shape/renderer mismatch draws an in-band error rather than panicking —
/// a misconfigured widget should show what's wrong, not crash the splash.
pub fn render_payload(
    frame: &mut Frame,
    area: Rect,
    payload: &Payload,
    spec: Option<&RenderSpec>,
    registry: &Registry,
) {
    let shape = shape_of(&payload.body);
    let (name, options) = match spec {
        Some(s) => (s.renderer_name().to_string(), s.options()),
        None => (
            default_renderer_for(shape).to_string(),
            RenderOptions::default(),
        ),
    };
    let Some(renderer) = registry.get(&name) else {
        render_error(frame, area, &format!("unknown renderer: {name}"));
        return;
    };
    if !renderer.accepts().contains(&shape) {
        render_error(
            frame,
            area,
            &format!("renderer {name} cannot display {shape:?}"),
        );
        return;
    }
    renderer.render(frame, area, &payload.body, &options);
}

/// `true` if any widget in `widgets` will be rendered by a renderer that declares itself
/// animated. Runtime uses this to decide whether to run a multi-frame render loop even when
/// load is fast (cache-first).
pub fn any_widget_animates(widgets: &[crate::config::WidgetConfig], registry: &Registry) -> bool {
    widgets.iter().any(|w| {
        let name = match &w.render {
            Some(spec) => spec.renderer_name(),
            // Widgets without an explicit renderer use the shape default, which is always a
            // static renderer in the current builtin set.
            None => return false,
        };
        registry.get(name).is_some_and(|r| r.animates())
    })
}

fn render_error(frame: &mut Frame, area: Rect, msg: &str) {
    frame.render_widget(Paragraph::new(msg), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Wrapper {
        render: RenderSpec,
    }

    #[test]
    fn short_form_parses_as_renderer_name() {
        let w: Wrapper = toml::from_str(r#"render = "simple""#).unwrap();
        assert_eq!(w.render.renderer_name(), "simple");
        assert!(w.render.options().pixel_size.is_none());
    }

    #[test]
    fn full_form_carries_options() {
        let w: Wrapper =
            toml::from_str(r#"render = { type = "ascii_art", pixel_size = "quadrant" }"#).unwrap();
        assert_eq!(w.render.renderer_name(), "ascii_art");
        assert_eq!(w.render.options().pixel_size.as_deref(), Some("quadrant"));
    }

    #[test]
    fn registry_resolves_all_builtins() {
        let r = Registry::with_builtins();
        for name in [
            "simple",
            "ascii_art",
            "animated_typewriter",
            "list",
            "table",
            "gauge",
            "line_gauge",
            "sparkline",
            "chart_line",
            "scatter",
            "bar_chart",
            "image",
            "calendar",
        ] {
            assert!(r.get(name).is_some(), "missing builtin renderer: {name}");
        }
    }

    #[test]
    fn default_renderer_covers_every_shape() {
        for s in [
            Shape::Lines,
            Shape::Entries,
            Shape::Ratio,
            Shape::NumberSeries,
            Shape::PointSeries,
            Shape::Bars,
            Shape::Image,
            Shape::Calendar,
        ] {
            let name = default_renderer_for(s);
            let r = Registry::with_builtins();
            let renderer = r.get(name).unwrap_or_else(|| panic!("no renderer {name}"));
            assert!(
                renderer.accepts().contains(&s),
                "default renderer {name} doesn't accept its shape {s:?}"
            );
        }
    }

    #[test]
    fn lines_shape_has_multiple_renderers() {
        // The point of the whole registry: one shape, many renderers. Lines is consumed by both
        // the plain `simple` renderer and the `ascii_art` renderer — users pick via config.
        let r = Registry::with_builtins();
        assert!(r.get("simple").unwrap().accepts().contains(&Shape::Lines));
        assert!(
            r.get("ascii_art")
                .unwrap()
                .accepts()
                .contains(&Shape::Lines)
        );
    }
}
