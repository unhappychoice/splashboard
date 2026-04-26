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
mod list_plain;
mod list_timeline;
pub mod loading;
mod media_image;
mod status_badge;
mod text_ascii;
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

/// Per-renderer configuration. Each renderer picks out the fields it cares about from this bag;
/// others are ignored. Kept deliberately flat so config authors can write
/// `render = { type = "text_ascii", style = "figlet" }` without nesting.
///
/// Renderer naming: every renderer carries a family prefix (`text_*`, `list_*`, `chart_*`,
/// `gauge_*`, `grid_*`, `status_*`, `media_*`, `animated_*`, `clock_*`), never a suffix.
/// Module name always matches the registered name (`chart_bar.rs` registers `"chart_bar"`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderOptions {
    /// Visual treatment selector. Shared across renderers where "same shape, different look"
    /// makes sense: `text_ascii` ("blocks" / "figlet"), `chart_sparkline` ("bars" / "line").
    #[serde(default)]
    pub style: Option<String>,
    /// text_ascii blocks style: "full" | "quadrant" | "sextant". None = adaptive (pick by area
    /// height). Ignored by other styles.
    #[serde(default)]
    pub pixel_size: Option<String>,
    /// Horizontal alignment of the rendered content within its cell: "left" (default),
    /// "center", or "right". Not every renderer honours this — text_plain and text_ascii do;
    /// structural renderers (grid_table, gauge_*, chart_*) ignore it.
    #[serde(default)]
    pub align: Option<String>,
    /// text_ascii figlet font name. Built-ins: "standard" (default), "small", "big", "banner".
    /// Ignored when `style != "figlet"`.
    #[serde(default)]
    pub font: Option<String>,
    /// Theme token name overriding the renderer's default foreground colour. Any
    /// `[theme]` key is accepted; unknown names fall back to the renderer's default.
    /// Honoured by `text_ascii` and `status_badge`.
    #[serde(default)]
    pub color: Option<String>,
    /// media_image: object fit mode. "contain" (default, preserves aspect inside box),
    /// "cover" (fill box, crop overflow), "stretch" (legacy — warp to box).
    #[serde(default)]
    pub fit: Option<String>,
    /// media_image: upper bound on the image cell width. None = no cap beyond the area.
    #[serde(default)]
    pub max_width: Option<u16>,
    /// media_image: upper bound on the image cell height. None = no cap beyond the area.
    #[serde(default)]
    pub max_height: Option<u16>,
    /// media_image / status_badge: inner padding in cells. `media_image` trims uniformly on
    /// all sides; `status_badge` uses it as horizontal padding before / after the label.
    #[serde(default)]
    pub padding: Option<u16>,
    /// gauge_line / chart_sparkline: inline label prefix. `gauge_line` renders as
    /// `label: ▓▓░░ 32%`; `chart_sparkline` as `label  ▁▂▃█`.
    #[serde(default)]
    pub label: Option<String>,
    /// gauge_line: numeric formatting. "percent" (default, `32%`), "fraction"
    /// (`118 of 365`), or "both" (`32% (118 of 365)`). "fraction" / "both" need
    /// `RatioData.denominator` populated by the fetcher — otherwise they fall back
    /// to percent.
    #[serde(default)]
    pub value_format: Option<String>,
    /// grid_table: layout mode. "rows" (default — one row per entry) or "inline"
    /// (single-line `key: value · key: value`).
    #[serde(default)]
    pub layout: Option<String>,
    /// grid_table: per-column alignment. First entry aligns the key column, second the
    /// value column. Valid values: "left" (default) / "center" / "right".
    #[serde(default)]
    pub column_align: Option<Vec<String>>,
    /// list_plain / list_timeline: bullet glyph prefixed to each entry. "•", "●", "▪",
    /// "→", or "none" (omit). None leaves list_plain unbulleted, list_timeline at its
    /// default "●".
    #[serde(default)]
    pub bullet: Option<String>,
    /// list_plain / list_timeline: cap on rendered entries. None = render all.
    #[serde(default)]
    pub max_items: Option<usize>,
    /// list_timeline: how the time prefix is formatted. "relative" (default, `2h`) or
    /// "absolute" (`2026-04-23`).
    #[serde(default)]
    pub date_format: Option<String>,
    /// status_badge: pill shape. "rounded" (default, `( label )`), "square" (`[ label ]`),
    /// or "none" (bare — keeps the coloured dot prefix).
    #[serde(default)]
    pub shape: Option<String>,
    /// chart_pie: legend placement. "right" (default) / "bottom" / "none".
    #[serde(default)]
    pub legend: Option<String>,
    /// chart_pie: render as a donut (empty hole in the centre) instead of a full pie.
    #[serde(default)]
    pub donut: Option<bool>,
    /// chart_bar: orient bars horizontally instead of the default vertical.
    #[serde(default)]
    pub horizontal: Option<bool>,
    /// chart_bar: stack series on top of each other (noop for single-series data).
    #[serde(default)]
    pub stacked: Option<bool>,
    /// chart_bar: cap on rendered bars (keeps the top N by value). None = render all.
    #[serde(default)]
    pub max_bars: Option<usize>,
    /// chart_histogram: number of buckets the input series is binned into. None defaults to
    /// an auto value derived from the visible width and the sample count.
    #[serde(default)]
    pub bins: Option<u16>,
    /// chart_histogram: render the value range (min / max) on the bottom edge and the peak
    /// count on the top edge. Skipped automatically when the chart area is shorter than 4
    /// rows so the bars themselves still get usable space.
    #[serde(default)]
    pub axis_labels: Option<bool>,
    /// chart_bar: thickness of each bar, in cells. Applied along the axis perpendicular to
    /// the bar direction (bar height for vertical, bar row-height for horizontal). None
    /// defaults to `3` for vertical and `1` for horizontal — horizontal bars read fine
    /// single-row and wrapping several into a compact slot is usually what the user wants.
    #[serde(default)]
    pub bar_width: Option<u16>,
    /// chart_line / chart_scatter: draw a bordered axis block around the plot.
    #[serde(default)]
    pub axes: Option<bool>,
    /// chart_scatter: scatter glyph. "dot" (default, `●`), "cross" (`✕`), "plus" (`+`),
    /// or any single printable character (used verbatim).
    /// grid_calendar: event marker glyph, same accepted values.
    #[serde(default)]
    pub marker: Option<String>,
    /// gauge_battery / gauge_segment: how the fill colour follows `Ratio.value`. "neutral"
    /// (default) is single `theme.text`. "fill" treats the value as how-full (low →
    /// status_error, high → status_ok) — right for battery / quota progress. "drain" inverts
    /// (high → status_error) — right for disk / memory / cpu where the ratio is "fraction used".
    #[serde(default)]
    pub tone: Option<String>,
    /// gauge_segment: number of LED segments rendered. None defaults to 5 — the canonical
    /// "5-LED bar" silhouette. Values are clamped to at least 1.
    #[serde(default)]
    pub segments: Option<u16>,
    /// gauge_circle: thickness of the ring in cells. None keeps the block-gauge default.
    #[serde(default)]
    pub ring_thickness: Option<u16>,
    /// gauge_circle: placement of the numeric label. "center" (default) or "below".
    #[serde(default)]
    pub label_position: Option<String>,
    /// grid_heatmap: glyph painted at every cell. Single character (e.g. "■", "●", "▮").
    /// None keeps the density-ramp glyph set.
    #[serde(default)]
    pub cell_char: Option<String>,
    /// grid_calendar: which day starts the week. "sun" (default), "mon".
    #[serde(default)]
    pub week_start: Option<String>,
    /// animated_postfx: name of the inner renderer whose output the effect is applied over.
    /// None falls back to the shape's default renderer.
    #[serde(default)]
    pub inner: Option<String>,
    /// animated_postfx: effect to apply. Names map to tachyonfx effects (`fade_in`, `fade_out`,
    /// `dissolve`, `coalesce`, `sweep_in`, `slide_in`, `hsl_shift`, `glitch`). None defaults to
    /// `fade_in`. Unknown names fall back to `fade_in` rather than failing the widget.
    #[serde(default)]
    pub effect: Option<String>,
    /// animated_postfx: effect duration in milliseconds. None defaults to 800 — short enough
    /// that the effect completes well within the 2s ANIMATION_WINDOW, leaving the final frame
    /// static for the remainder.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// animated_figlet_morph: ordered list of figlet font names to step through. Each phase
    /// takes `duration_ms / N` and crossfades into the next; the final entry is the resting
    /// font once the animation completes. None falls back to `["small", "banner", "ansi_shadow"]`.
    #[serde(default)]
    pub font_sequence: Option<Vec<String>>,
    /// animated_boot: ordered list of boot-log lines scrolled in one by one before the hero
    /// settles. None uses a small default set; an empty list disables the boot lines and
    /// falls straight to the inner render.
    #[serde(default)]
    pub boot_lines: Option<Vec<String>>,
}

/// What the TOML accepts for `render`. Short form `render = "text_plain"` uses defaults; long
/// form `render = { type = "text_ascii", pixel_size = "quadrant" }` carries options. Absence
/// means "use the default renderer for this shape".
///
/// The `Full` variant is noticeably bigger than `Short` — [`RenderOptions`] is a flat bag of
/// per-renderer fields so a growing catalog expands it. Config-parsed objects live for one
/// widget's draw path; boxing the variant just to shrink the enum adds an indirection without
/// measurable win, so the size difference is intentional.
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
        r.register(Arc::new(animated_typewriter::AnimatedTypewriterRenderer));
        r.register(Arc::new(animated_postfx::AnimatedPostfxRenderer));
        r.register(Arc::new(animated_figlet_morph::AnimatedFigletMorphRenderer));
        r.register(Arc::new(animated_boot::AnimatedBootRenderer));
        r.register(Arc::new(animated_scanlines::AnimatedScanlinesRenderer));
        r.register(Arc::new(animated_splitflap::AnimatedSplitflapRenderer));
        r.register(Arc::new(animated_wave::AnimatedWaveRenderer));
        r.register(Arc::new(status_badge::StatusBadgeRenderer));
        r.register(Arc::new(list_plain::ListPlainRenderer));
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
        Shape::TextBlock => "text_plain",
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
        assert!(w.render.options().pixel_size.is_none());
    }

    #[test]
    fn full_form_carries_options() {
        let w: Wrapper =
            toml::from_str(r#"render = { type = "text_ascii", pixel_size = "quadrant" }"#).unwrap();
        assert_eq!(w.render.renderer_name(), "text_ascii");
        assert_eq!(w.render.options().pixel_size.as_deref(), Some("quadrant"));
    }

    #[test]
    fn full_form_carries_postfx_options() {
        let w: Wrapper = toml::from_str(
            r#"render = { type = "animated_postfx", inner = "text_ascii", effect = "dissolve", duration_ms = 1200 }"#,
        )
        .unwrap();
        let opts = w.render.options();
        assert_eq!(w.render.renderer_name(), "animated_postfx");
        assert_eq!(opts.inner.as_deref(), Some("text_ascii"));
        assert_eq!(opts.effect.as_deref(), Some("dissolve"));
        assert_eq!(opts.duration_ms, Some(1200));
    }

    #[test]
    fn registry_resolves_all_builtins() {
        let r = Registry::with_builtins();
        for name in [
            "text_plain",
            "text_ascii",
            "animated_typewriter",
            "animated_postfx",
            "animated_figlet_morph",
            "animated_boot",
            "animated_scanlines",
            "animated_splitflap",
            "animated_wave",
            "status_badge",
            "list_plain",
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
