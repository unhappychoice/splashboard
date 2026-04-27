use std::collections::HashMap;
use std::sync::Arc;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::Paragraph,
};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::theme::{ColorKey, Theme};

mod animated_boot;
mod animated_figlet_morph;
mod animated_postfx;
mod animated_scanlines;
mod animated_splitflap;
mod animated_typewriter;
mod animated_wave;
mod chart_bar;
mod chart_histogram;
mod chart_line;
mod chart_pie;
mod chart_scatter;
mod chart_sparkline;
mod gauge_battery;
mod gauge_circle;
mod gauge_line;
mod gauge_segment;
mod gauge_thermometer;
mod grid_calendar;
mod grid_heatmap;
mod grid_table;
pub mod list_links;
mod list_plain;
mod list_ranking;
mod list_timeline;
pub mod loading;
mod media_image;
mod status_badge;
mod text_ascii;
mod text_markdown;
mod text_plain;

#[cfg(test)]
pub mod test_utils;

/// Data shape a fetcher produces. A shape is consumed by one or more renderers — a `Text`
/// shape can be rendered by `text_plain` (plain text), `text_ascii` (big-text via tui-big-text),
/// and future alternatives (figlet, animated typewriter) without the fetcher needing to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Text,
    TextBlock,
    MarkdownTextBlock,
    LinkedTextBlock,
    Entries,
    Ratio,
    NumberSeries,
    PointSeries,
    Bars,
    Image,
    Calendar,
    Heatmap,
    Badge,
    Timeline,
}

pub fn shape_of(body: &Body) -> Shape {
    match body {
        Body::Text(_) => Shape::Text,
        Body::TextBlock(_) => Shape::TextBlock,
        Body::MarkdownTextBlock(_) => Shape::MarkdownTextBlock,
        Body::LinkedTextBlock(_) => Shape::LinkedTextBlock,
        Body::Entries(_) => Shape::Entries,
        Body::Ratio(_) => Shape::Ratio,
        Body::NumberSeries(_) => Shape::NumberSeries,
        Body::PointSeries(_) => Shape::PointSeries,
        Body::Bars(_) => Shape::Bars,
        Body::Image(_) => Shape::Image,
        Body::Calendar(_) => Shape::Calendar,
        Body::Heatmap(_) => Shape::Heatmap,
        Body::Badge(_) => Shape::Badge,
        Body::Timeline(_) => Shape::Timeline,
    }
}

impl Shape {
    /// Stable snake_case label. Used in cache keys and error messages; kept short so cache-key
    /// lengths stay reasonable.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::TextBlock => "text_block",
            Self::MarkdownTextBlock => "markdown_text_block",
            Self::LinkedTextBlock => "linked_text_block",
            Self::Entries => "entries",
            Self::Ratio => "ratio",
            Self::NumberSeries => "number_series",
            Self::PointSeries => "point_series",
            Self::Bars => "bars",
            Self::Image => "image",
            Self::Calendar => "calendar",
            Self::Heatmap => "heatmap",
            Self::Badge => "badge",
            Self::Timeline => "timeline",
        }
    }
}

/// Per-renderer configuration. Splits cross-cutting fields (used by ≥2 renderers across
/// families) from per-renderer extras: shared fields are explicit on this struct; the rest
/// land in [`RenderOptions::raw`] and each renderer parses its own typed Options struct via
/// [`RenderOptions::parse_specific`]. Mirrors the fetcher pattern (`FetchContext.options:
/// Option<toml::Value>` parsed into per-fetcher `Options` structs).
///
/// Config authors keep writing the flat form — `render = { type = "text_ascii", style =
/// "figlet", font = "small" }` — TOML `flatten` collects unknown keys into `raw`. Typo
/// detection on renderer-specific fields is delegated to each renderer's own
/// `#[serde(deny_unknown_fields)]` Options struct.
///
/// Renderer naming: every renderer carries a family prefix (`text_*`, `list_*`, `chart_*`,
/// `gauge_*`, `grid_*`, `status_*`, `media_*`, `animated_*`, `clock_*`), never a suffix.
/// Module name always matches the registered name (`chart_bar.rs` registers `"chart_bar"`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RenderOptions {
    /// Visual treatment selector. Shared across renderers where "same shape, different look"
    /// makes sense: `text_ascii` ("blocks" / "figlet"), `chart_sparkline` ("bars" / "line"),
    /// `list_ranking` ("medal" / "number" / "none").
    #[serde(default)]
    pub style: Option<String>,
    /// Horizontal alignment of the rendered content within its cell: "left" (default),
    /// "center", or "right". Not every renderer honours this — text_plain and text_ascii do;
    /// structural renderers (grid_table, gauge_*, chart_*) ignore it.
    #[serde(default)]
    pub align: Option<String>,
    /// Theme token name overriding the renderer's default foreground colour. Any
    /// `[theme]` key is accepted; unknown names fall back to the renderer's default.
    /// Honoured by `text_ascii` and `animated_figlet_morph`.
    #[serde(default)]
    pub color: Option<String>,
    /// Inline label prefix used by gauge / chart families. `gauge_line` renders as
    /// `label: ▓▓░░ 32%`; `chart_sparkline` as `label  ▁▂▃█`; `chart_histogram` as the
    /// chart title.
    #[serde(default)]
    pub label: Option<String>,
    /// Inner padding in cells. `media_image` trims uniformly on all sides; `status_badge` uses
    /// it as horizontal padding before / after the label.
    #[serde(default)]
    pub padding: Option<u16>,
    /// list_plain / list_timeline: bullet glyph prefixed to each entry. "•", "●", "▪",
    /// "→", or "none" (omit). None leaves list_plain unbulleted, list_timeline at its
    /// default "●".
    #[serde(default)]
    pub bullet: Option<String>,
    /// list_*: cap on rendered entries. None = render all.
    #[serde(default)]
    pub max_items: Option<usize>,
    /// gauge_*: how the fill colour follows `Ratio.value`. "neutral" (default) is single
    /// `theme.text`. "fill" treats the value as how-full (low → status_error, high →
    /// status_ok) — right for battery / quota progress. "drain" inverts (high → status_error)
    /// — right for disk / memory / cpu where the ratio is "fraction used".
    #[serde(default)]
    pub tone: Option<String>,
    /// gauge_*: numeric formatting. "percent" (default, `32%`), "fraction" (`118 of 365`),
    /// or "both" (`32% (118 of 365)`). "fraction" / "both" need `RatioData.denominator`
    /// populated by the fetcher — otherwise they fall back to percent.
    #[serde(default)]
    pub value_format: Option<String>,
    /// animated_*: effect duration in milliseconds. Default per-renderer (typically 800ms,
    /// short enough that the effect completes well within the 2s ANIMATION_WINDOW).
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// chart_line / chart_scatter: draw a bordered axis block around the plot.
    #[serde(default)]
    pub axes: Option<bool>,
    /// Renderer-specific extras. Each renderer parses its own typed Options struct via
    /// [`Self::parse_specific`]; unknown keys for that renderer surface as parse errors at
    /// render time. Common-field typos (e.g. `aling = "center"`) also land here, then get
    /// rejected by whichever renderer the spec routes to.
    ///
    /// `pub` so functional-record-update (`..RenderOptions::default()`) compiles in tests
    /// and the `xtask` crate's preview/snapshot helpers. Mutating callers should still go
    /// through [`Self::with_extra`] / [`Self::without_extra`] rather than touching the map
    /// directly.
    #[serde(flatten, default)]
    pub raw: toml::Table,
}

impl RenderOptions {
    /// Parse the renderer-specific subset of `raw` into the renderer's own typed Options
    /// struct. Returns the type's `Default` on parse failure — keeps the trait sig free of
    /// `Result` while staying lenient (matches the existing "unknown keys are warnings, not
    /// crashes" posture). Per-renderer Options should still derive
    /// `#[serde(deny_unknown_fields)]` so structural typos surface in tests.
    pub fn parse_specific<T: serde::de::DeserializeOwned + Default>(&self) -> T {
        if self.raw.is_empty() {
            return T::default();
        }
        toml::Value::Table(self.raw.clone())
            .try_into()
            .unwrap_or_default()
    }

    /// Builder helper for tests and animation-wrapper renderers that need to forward a
    /// modified options bag to an inner renderer. Inserts (or overwrites) a key in `raw` and
    /// serialises the value through TOML so the field round-trips through the same path
    /// runtime config takes.
    pub fn with_extra<V: serde::Serialize>(mut self, key: &str, value: V) -> Self {
        if let Ok(v) = toml::Value::try_from(value) {
            self.raw.insert(key.to_string(), v);
        }
        self
    }

    /// Drops a renderer-specific key from `raw` so wrappers like `animated_postfx` can strip
    /// their own options before forwarding to the inner renderer.
    pub fn without_extra(mut self, key: &str) -> Self {
        self.raw.remove(key);
        self
    }

    /// Read a renderer-specific extra as `&str`. Convenience for the few callers that only
    /// need a single field and don't want to spin up a typed Options struct (preview /
    /// dashboard_snapshot peek at `inner`, for instance).
    pub fn extra_str(&self, key: &str) -> Option<&str> {
        self.raw.get(key).and_then(|v| v.as_str())
    }

    /// Read a renderer-specific extra as a list of strings. Mirrors [`Self::extra_str`] for
    /// array values like `font_sequence`.
    pub fn extra_string_array(&self, key: &str) -> Option<Vec<String>> {
        self.raw.get(key).and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
    }
}

/// What the TOML accepts for `render`. Short form `render = "text_plain"` uses defaults; long
/// form `render = { type = "text_ascii", pixel_size = "quadrant" }` carries options. Absence
/// means "use the default renderer for this shape".
///
/// The `Full` variant is bigger than `Short` because it carries the (cross-cutting fields +
/// raw extras) [`RenderOptions`]. Config-parsed objects live for one widget's draw path;
/// boxing the variant just to shrink the enum adds an indirection without measurable win.
#[allow(clippy::large_enum_variant)]
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
    /// One- or two-sentence summary surfaced in the generated reference docs. Should describe
    /// the visual character (a hero block, a sparkline, a status pill) and, where multiple
    /// siblings exist within the same family (`text_plain` vs `text_ascii`, `gauge_circle` vs
    /// `gauge_segment`), what makes this one different. Read by config authors browsing the
    /// catalog for the right look.
    fn description(&self) -> &'static str;
    fn accepts(&self) -> &[Shape];
    /// Whether this renderer produces different output on repeated calls within one draw
    /// cycle. The runtime consults this: any true answer upgrades the draw phase from a single
    /// one-shot call into a time-boxed multi-frame loop so the animation actually plays. Load
    /// state (cache warm / cold / --wait) is not consulted — load and render are independent
    /// axes.
    fn animates(&self) -> bool {
        false
    }
    /// Options this renderer reads from the `render` inline table (e.g., `style`, `pixel_size`,
    /// `align`). Default is an empty slice — most renderers only read the shared fields on
    /// [`RenderOptions`]. Consumed at docs-generation time by the `xtask` crate.
    fn option_schemas(&self) -> &[OptionSchema] {
        &[]
    }
    /// Semantic colour tokens this renderer reads from the shared [`Theme`]. Declared here so
    /// the xtask docs generator can list, per renderer, exactly which `[theme]` keys affect it.
    /// Default is an empty slice — renderers that only draw structural glyphs don't consume
    /// colour.
    fn color_keys(&self) -> &[ColorKey] {
        &[]
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        registry: &Registry,
    );
    /// Height in cells the renderer would naturally use for `body` at the given
    /// `max_width`. Only consulted by layout when a row / column is sized
    /// `Auto`; most renderers stay with the default of `1` (single-line output)
    /// and let `Length` / `Fill` constraints drive their cell. `text_ascii`
    /// overrides this so figlet heroes report the wrapped block stack height;
    /// `animated_postfx` delegates to its inner renderer via `registry`.
    fn natural_height(
        &self,
        _body: &Body,
        _opts: &RenderOptions,
        _max_width: u16,
        _registry: &Registry,
    ) -> u16 {
        1
    }
}

#[derive(Default, Clone)]
pub struct Registry {
    renderers: HashMap<String, Arc<dyn Renderer>>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Arc::new(text_plain::TextPlainRenderer));
        r.register(Arc::new(text_ascii::TextAsciiRenderer));
        r.register(Arc::new(text_markdown::TextMarkdownRenderer));
        r.register(Arc::new(animated_typewriter::AnimatedTypewriterRenderer));
        r.register(Arc::new(animated_postfx::AnimatedPostfxRenderer));
        r.register(Arc::new(animated_figlet_morph::AnimatedFigletMorphRenderer));
        r.register(Arc::new(animated_boot::AnimatedBootRenderer));
        r.register(Arc::new(animated_scanlines::AnimatedScanlinesRenderer));
        r.register(Arc::new(animated_splitflap::AnimatedSplitflapRenderer));
        r.register(Arc::new(animated_wave::AnimatedWaveRenderer));
        r.register(Arc::new(status_badge::StatusBadgeRenderer));
        r.register(Arc::new(list_plain::ListPlainRenderer));
        r.register(Arc::new(list_links::ListLinksRenderer));
        r.register(Arc::new(list_ranking::ListRankingRenderer));
        r.register(Arc::new(grid_table::GridTableRenderer));
        r.register(Arc::new(gauge_battery::GaugeBatteryRenderer));
        r.register(Arc::new(gauge_circle::GaugeCircleRenderer));
        r.register(Arc::new(gauge_line::GaugeLineRenderer));
        r.register(Arc::new(gauge_segment::GaugeSegmentRenderer));
        r.register(Arc::new(gauge_thermometer::GaugeThermometerRenderer));
        r.register(Arc::new(chart_sparkline::ChartSparklineRenderer));
        r.register(Arc::new(chart_line::ChartLineRenderer));
        r.register(Arc::new(chart_scatter::ChartScatterRenderer));
        r.register(Arc::new(chart_bar::ChartBarRenderer));
        r.register(Arc::new(chart_histogram::ChartHistogramRenderer));
        r.register(Arc::new(chart_pie::ChartPieRenderer));
        r.register(Arc::new(media_image::MediaImageRenderer));
        r.register(Arc::new(grid_calendar::GridCalendarRenderer));
        r.register(Arc::new(grid_heatmap::GridHeatmapRenderer));
        r.register(Arc::new(list_timeline::ListTimelineRenderer));
        r
    }

    pub fn register(&mut self, r: Arc<dyn Renderer>) {
        self.renderers.insert(r.name().to_string(), r);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Renderer>> {
        self.renderers.get(name).cloned()
    }

    /// All registered renderers, sorted by name. Useful for deterministic docs generation.
    pub fn sorted(&self) -> Vec<Arc<dyn Renderer>> {
        let mut entries: Vec<_> = self.renderers.values().cloned().collect();
        entries.sort_by(|a, b| a.name().cmp(b.name()));
        entries
    }
}

pub fn default_renderer_for(shape: Shape) -> &'static str {
    match shape {
        Shape::Text => "text_plain",
        Shape::TextBlock => "list_plain",
        Shape::MarkdownTextBlock => "text_markdown",
        Shape::LinkedTextBlock => "list_links",
        Shape::Entries => "grid_table",
        Shape::Ratio => "gauge_circle",
        Shape::NumberSeries => "chart_sparkline",
        Shape::PointSeries => "chart_line",
        Shape::Bars => "chart_bar",
        Shape::Image => "media_image",
        Shape::Calendar => "grid_calendar",
        Shape::Heatmap => "grid_heatmap",
        Shape::Badge => "status_badge",
        Shape::Timeline => "list_timeline",
    }
}

/// Resolves a renderer by spec (or shape default), compat-checks it, and renders. An unknown
/// renderer name or a shape/renderer mismatch draws an in-band error rather than panicking —
/// a misconfigured widget should show what's wrong, not crash the splash.
///
/// Bodies that carry no usable data short-circuit to [`render_empty_state`] before dispatch so
/// every widget shows the same "nothing here yet" placeholder instead of each renderer's
/// individual no-op. Keeps user-written ReadStore widgets, and any widget whose upstream data
/// hasn't arrived yet, visually present rather than appearing as absent layout.
pub fn render_payload(
    frame: &mut Frame,
    area: Rect,
    payload: &Payload,
    spec: Option<&RenderSpec>,
    registry: &Registry,
    theme: &Theme,
) {
    if is_empty_body(&payload.body) {
        render_empty_state(frame, area, theme);
        return;
    }
    let shape = shape_of(&payload.body);
    let (name, options) = match spec {
        Some(s) => (s.renderer_name().to_string(), s.options()),
        None => (
            default_renderer_for(shape).to_string(),
            RenderOptions::default(),
        ),
    };
    let Some(renderer) = registry.get(&name) else {
        render_error(frame, area, &format!("unknown renderer: {name}"), theme);
        return;
    };
    if !renderer.accepts().contains(&shape) {
        render_error(
            frame,
            area,
            &format!("renderer {name} cannot display {shape:?}"),
            theme,
        );
        return;
    }
    renderer.render(frame, area, &payload.body, &options, theme, registry);
}

/// `true` when the body has no meaningful data to render. Callers (most usefully
/// [`render_payload`]) use this to substitute an empty-state placeholder rather than letting a
/// specific renderer silently draw nothing.
///
/// Note on specific shapes:
/// - `Ratio` is never considered empty (0.0 is legitimate, e.g. "0% disk used").
/// - `Calendar` is never considered empty (month/year are always present).
/// - Everything else checks its natural collection.
pub fn is_empty_body(body: &Body) -> bool {
    match body {
        Body::Text(d) => d.value.is_empty(),
        Body::TextBlock(d) => d.lines.is_empty() || d.lines.iter().all(|l| l.is_empty()),
        Body::MarkdownTextBlock(d) => d.value.is_empty(),
        Body::LinkedTextBlock(d) => d.items.is_empty() || d.items.iter().all(|i| i.text.is_empty()),
        Body::Entries(d) => d.items.is_empty(),
        Body::NumberSeries(d) => d.values.is_empty(),
        Body::PointSeries(d) => d.series.iter().all(|s| s.points.is_empty()),
        Body::Bars(d) => d.bars.is_empty(),
        Body::Image(d) => d.path.is_empty(),
        Body::Heatmap(d) => d.cells.is_empty() || d.cells.iter().all(|r| r.is_empty()),
        Body::Timeline(d) => d.events.is_empty(),
        Body::Badge(_) | Body::Calendar(_) | Body::Ratio(_) => false,
    }
}

fn render_empty_state(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // Two lines if there's room (icon + caption), otherwise collapse to the caption alone.
    // Centered both axes via a Fill / Length / Fill vertical layout.
    let (lines_src, min_width): (&[&str], u16) = if area.height >= 2 {
        (&["◌", "nothing here yet"], 16)
    } else {
        (&["◌ nothing here yet"], 18)
    };
    // Skip rendering when the caption can't fit — a narrow spacer (e.g. 2-cell `basic_static`
    // gap) should read as absent space, not leak truncated "no…" fragments into neighbors.
    if area.width < min_width {
        return;
    }
    let dim = Style::default()
        .fg(theme.text_dim)
        .add_modifier(Modifier::ITALIC);
    let lines: Vec<Line> = lines_src
        .iter()
        .map(|s| Line::from(*s).style(dim))
        .collect();
    let content_height = lines.len() as u16;
    let chunks = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(content_height),
        Constraint::Fill(1),
    ])
    .split(area);
    let p = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(p, chunks[1]);
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

fn render_error(frame: &mut Frame, area: Rect, msg: &str, theme: &Theme) {
    // Misconfiguration should pop, not blend — route through the status-error token so the
    // coral-red reads as "fix this" against the theme surface.
    frame.render_widget(
        Paragraph::new(msg).style(Style::default().fg(theme.status_error)),
        area,
    );
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
        let w: Wrapper = toml::from_str(r#"render = "text_plain""#).unwrap();
        assert_eq!(w.render.renderer_name(), "text_plain");
        assert!(w.render.options().align.is_none());
    }

    #[test]
    fn full_form_carries_common_options() {
        let w: Wrapper =
            toml::from_str(r#"render = { type = "text_plain", align = "center" }"#).unwrap();
        assert_eq!(w.render.renderer_name(), "text_plain");
        assert_eq!(w.render.options().align.as_deref(), Some("center"));
    }

    #[test]
    fn full_form_routes_specific_options_into_raw_blob() {
        // Renderer-specific keys ride through `raw` and are decoded by each renderer's own
        // typed Options struct via `parse_specific`. The wrapper sees `duration_ms` directly
        // (it's common across the animated family), and `inner` / `effect` only via parse.
        let w: Wrapper = toml::from_str(
            r#"render = { type = "animated_postfx", inner = "text_ascii", effect = "dissolve", duration_ms = 1200 }"#,
        )
        .unwrap();
        let opts = w.render.options();
        assert_eq!(w.render.renderer_name(), "animated_postfx");
        assert_eq!(opts.duration_ms, Some(1200));
        let parsed: animated_postfx::Options = opts.parse_specific();
        assert_eq!(parsed.inner.as_deref(), Some("text_ascii"));
        assert_eq!(parsed.effect.as_deref(), Some("dissolve"));
    }

    #[test]
    fn registry_resolves_all_builtins() {
        let r = Registry::with_builtins();
        for name in [
            "text_plain",
            "text_ascii",
            "text_markdown",
            "animated_typewriter",
            "animated_postfx",
            "animated_figlet_morph",
            "animated_boot",
            "animated_scanlines",
            "animated_splitflap",
            "animated_wave",
            "status_badge",
            "list_plain",
            "list_links",
            "list_ranking",
            "list_timeline",
            "grid_table",
            "grid_heatmap",
            "grid_calendar",
            "gauge_battery",
            "gauge_circle",
            "gauge_line",
            "gauge_segment",
            "gauge_thermometer",
            "chart_sparkline",
            "chart_line",
            "chart_scatter",
            "chart_bar",
            "chart_histogram",
            "chart_pie",
            "media_image",
        ] {
            assert!(r.get(name).is_some(), "missing builtin renderer: {name}");
        }
    }

    #[test]
    fn default_renderer_covers_every_shape() {
        for s in [
            Shape::Text,
            Shape::TextBlock,
            Shape::MarkdownTextBlock,
            Shape::Entries,
            Shape::Ratio,
            Shape::NumberSeries,
            Shape::PointSeries,
            Shape::Bars,
            Shape::Image,
            Shape::Calendar,
            Shape::Heatmap,
            Shape::Badge,
            Shape::Timeline,
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
    fn is_empty_body_matches_expectations() {
        use crate::payload::{
            BarsData, EntriesData, HeatmapData, ImageData, NumberSeriesData, PointSeriesData,
            RatioData, TextBlockData, TextData,
        };
        // Empty cases.
        assert!(is_empty_body(&Body::Text(TextData {
            value: String::new(),
        })));
        assert!(is_empty_body(&Body::TextBlock(TextBlockData {
            lines: vec![],
        })));
        assert!(is_empty_body(&Body::TextBlock(TextBlockData {
            lines: vec![String::new()],
        })));
        assert!(is_empty_body(&Body::Entries(EntriesData { items: vec![] })));
        assert!(is_empty_body(&Body::NumberSeries(NumberSeriesData {
            values: vec![],
        })));
        assert!(is_empty_body(&Body::PointSeries(PointSeriesData {
            series: vec![],
        })));
        assert!(is_empty_body(&Body::Bars(BarsData { bars: vec![] })));
        assert!(is_empty_body(&Body::Image(ImageData {
            path: String::new(),
        })));
        assert!(is_empty_body(&Body::Heatmap(HeatmapData {
            cells: vec![],
            thresholds: None,
            row_labels: None,
            col_labels: None,
        })));
        // Non-empty and "structurally always present" cases.
        assert!(!is_empty_body(&Body::Text(TextData { value: "x".into() })));
        assert!(!is_empty_body(&Body::TextBlock(TextBlockData {
            lines: vec!["x".into()],
        })));
        assert!(!is_empty_body(&Body::Ratio(RatioData {
            value: 0.0,
            label: None,
            denominator: None,
        })));
    }

    #[test]
    fn empty_body_renders_placeholder_not_specific_renderer() {
        use crate::payload::{HeatmapData, Payload};
        use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Heatmap(HeatmapData {
                cells: vec![],
                thresholds: None,
                row_labels: None,
                col_labels: None,
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("grid_heatmap".into());
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 40, 5);
        let joined: String = (0..5).map(|y| line_text(&buf, y)).collect();
        assert!(
            joined.contains("nothing here yet"),
            "missing caption: {joined:?}"
        );
    }

    #[test]
    fn empty_placeholder_skipped_when_area_too_narrow() {
        // Regression: 2-cell `basic_static` gap widgets used as horizontal spacers were
        // rendering a truncated "no…" fragment from the "nothing here yet" caption, spilling
        // the artifact into the neighboring widget's column.
        use crate::payload::{Payload, TextData};
        use crate::render::test_utils::{line_text, render_to_buffer_with_spec};
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                value: String::new(),
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("text_plain".into());
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 2, 6);
        let joined: String = (0..6).map(|y| line_text(&buf, y)).collect();
        assert!(
            !joined.contains("no") && !joined.contains('◌'),
            "narrow area should render nothing, got: {joined:?}"
        );
    }

    #[test]
    fn text_shape_has_multiple_renderers() {
        // The point of the whole registry: one shape, many renderers. `Text` is consumed by both
        // the plain `text_plain` renderer and the `text_ascii` renderer — users pick via config.
        let r = Registry::with_builtins();
        assert!(
            r.get("text_plain")
                .unwrap()
                .accepts()
                .contains(&Shape::Text)
        );
        assert!(
            r.get("text_ascii")
                .unwrap()
                .accepts()
                .contains(&Shape::Text)
        );
    }
}
