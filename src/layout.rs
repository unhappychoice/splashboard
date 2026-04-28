use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{
        Alignment, Constraint, Direction as RatDir, Flex as RatFlex, Layout as RatLayout, Rect,
    },
    style::{Color, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
};

use crate::config::General;
use crate::payload::Payload;
use crate::render::{Registry, RenderSpec, Shape, loading::render_loading, render_payload};
use crate::theme::Theme;

pub type WidgetId = String;

#[derive(Debug, Clone)]
pub enum Layout {
    Stack {
        direction: Direction,
        children: Vec<Child>,
        panel: Option<Panel>,
        flex: Flex,
        bg: BgLevel,
    },
    Widget {
        id: WidgetId,
        panel: Option<Panel>,
        bg: BgLevel,
    },
    /// Reserves its slot but paints nothing. Used by config spacers
    /// (`[[row.child]]` without a `widget`, or `[[row]] gap = N` between siblings).
    Empty,
}

/// Which semantic background a row / widget paints behind its content. `Default` is a no-op
/// (inherits whatever the viewport already has, typically `theme.bg`); `Subtle` paints
/// `theme.bg_subtle` so a config can split e.g. a header band from the main content band.
/// Both resolve against the theme at draw time rather than carrying a literal colour, so
/// swapping presets gives a consistent hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BgLevel {
    #[default]
    Default,
    Subtle,
}

/// How children are distributed along the stack's main axis when their constraints don't fill
/// the available space. `Legacy` is ratatui's default (proportional fill with Fill constraints);
/// `Center` / `Start` / `End` / `SpaceBetween` etc. take a narrower set of children and place
/// them within the parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Flex {
    #[default]
    Legacy,
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone)]
pub struct Child {
    pub size: Size,
    pub layout: Layout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Fill(u16),
    Length(u16),
    Min(u16),
    #[allow(dead_code)]
    Max(u16),
    #[allow(dead_code)]
    Percentage(u16),
    /// Height / width chosen from the widget's natural size at render time.
    /// Resolves by asking the renderer via `natural_height`; the layout engine
    /// substitutes a `Length(n)` constraint once `n` is known.
    Auto,
}

#[derive(Debug, Clone, Default)]
pub struct Panel {
    pub title: Option<String>,
    pub border: BorderStyle,
    pub title_align: TitleAlign,
}

/// Where the panel's title sits along its top rule. Only meaningful when the panel has a
/// `title` and a visible border (any `BorderStyle` except `None`'s absence-of-panel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BorderStyle {
    #[default]
    Plain,
    #[allow(dead_code)]
    Rounded,
    #[allow(dead_code)]
    Thick,
    #[allow(dead_code)]
    Double,
    /// Single top edge only, plain style. Used to paint a section divider above a row so a
    /// `title = "..."` hangs off the top rule — no side / bottom chrome is drawn.
    Top,
}

impl Layout {
    pub fn rows(children: Vec<Child>) -> Self {
        Self::Stack {
            direction: Direction::Vertical,
            children,
            panel: None,
            flex: Flex::Legacy,
            bg: BgLevel::Default,
        }
    }

    pub fn cols(children: Vec<Child>) -> Self {
        Self::Stack {
            direction: Direction::Horizontal,
            children,
            panel: None,
            flex: Flex::Legacy,
            bg: BgLevel::Default,
        }
    }

    pub fn flexed(self, flex: Flex) -> Self {
        match self {
            Self::Stack {
                direction,
                children,
                panel,
                flex: _,
                bg,
            } => Self::Stack {
                direction,
                children,
                panel,
                flex,
                bg,
            },
            other => other,
        }
    }

    pub fn widget(id: impl Into<WidgetId>) -> Self {
        Self::Widget {
            id: id.into(),
            panel: None,
            bg: BgLevel::Default,
        }
    }

    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self::Empty
    }

    pub fn titled(self, title: impl Into<String>) -> Self {
        self.with_panel(|p| p.title(title))
    }

    pub fn title_aligned(self, align: TitleAlign) -> Self {
        self.with_panel(|p| p.title_align(align))
    }

    #[allow(dead_code)]
    pub fn bordered(self, border: BorderStyle) -> Self {
        self.with_panel(|p| p.border(border))
    }

    pub fn with_bg(self, level: BgLevel) -> Self {
        match self {
            Self::Stack {
                direction,
                children,
                panel,
                flex,
                bg: _,
            } => Self::Stack {
                direction,
                children,
                panel,
                flex,
                bg: level,
            },
            Self::Widget { id, panel, bg: _ } => Self::Widget {
                id,
                panel,
                bg: level,
            },
            Self::Empty => Self::Empty,
        }
    }

    fn with_panel(self, f: impl FnOnce(Panel) -> Panel) -> Self {
        match self {
            Self::Stack {
                direction,
                children,
                panel,
                flex,
                bg,
            } => {
                let p = f(panel.unwrap_or_default());
                Self::Stack {
                    direction,
                    children,
                    panel: Some(p),
                    flex,
                    bg,
                }
            }
            Self::Widget { id, panel, bg } => {
                let p = f(panel.unwrap_or_default());
                Self::Widget {
                    id,
                    panel: Some(p),
                    bg,
                }
            }
            Self::Empty => Self::Empty,
        }
    }
}

impl Panel {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn border(mut self, border: BorderStyle) -> Self {
        self.border = border;
        self
    }

    pub fn title_align(mut self, align: TitleAlign) -> Self {
        self.title_align = align;
        self
    }
}

impl Child {
    pub fn fill(weight: u16, layout: Layout) -> Self {
        Self {
            size: Size::Fill(weight),
            layout,
        }
    }

    pub fn length(rows_or_cols: u16, layout: Layout) -> Self {
        Self {
            size: Size::Length(rows_or_cols),
            layout,
        }
    }

    pub fn min(rows_or_cols: u16, layout: Layout) -> Self {
        Self {
            size: Size::Min(rows_or_cols),
            layout,
        }
    }

    #[allow(dead_code)]
    pub fn max(rows_or_cols: u16, layout: Layout) -> Self {
        Self {
            size: Size::Max(rows_or_cols),
            layout,
        }
    }

    pub fn auto(layout: Layout) -> Self {
        Self {
            size: Size::Auto,
            layout,
        }
    }

    #[allow(dead_code)]
    pub fn percentage(percent: u16, layout: Layout) -> Self {
        Self {
            size: Size::Percentage(percent),
            layout,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    area: Rect,
    layout: &Layout,
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
    theme: &Theme,
    general: &General,
    loading: &HashMap<WidgetId, Shape>,
) {
    match layout {
        Layout::Stack {
            direction,
            children,
            panel,
            flex,
            bg,
        } => {
            paint_bg(frame, area, *bg, theme);
            let inner = draw_panel(frame, area, panel, theme);
            draw_stack(
                frame, inner, *direction, *flex, children, widgets, specs, registry, theme,
                general, loading,
            );
        }
        Layout::Widget { id, panel, bg } => {
            paint_bg(frame, area, *bg, theme);
            let inner = draw_panel(frame, area, panel, theme);
            // Loading placeholder takes precedence over `widgets.get(id)` so widgets whose
            // cache entries haven't landed yet show a spinner instead of leaving a blank slot.
            // The daemon swaps the loading state off for that id once the real payload arrives.
            if let Some(&shape) = loading.get(id) {
                render_loading(frame, inner, shape, theme);
            } else if let Some(payload) = widgets.get(id) {
                render_payload(
                    frame,
                    inner,
                    payload,
                    specs.get(id),
                    registry,
                    theme,
                    general,
                );
            }
        }
        Layout::Empty => {}
    }
}

/// Fills `area` with the theme colour corresponding to `level`. `Default` is a no-op (cells
/// stay whatever the parent/viewport painted); `Subtle` paints `theme.bg_subtle`. A
/// themed-subtle resolving to `Color::Reset` also no-ops so users opting into `bg = "subtle"`
/// without a matching theme override don't accidentally clear existing cells.
fn paint_bg(frame: &mut Frame, area: Rect, level: BgLevel, theme: &Theme) {
    let color = match level {
        BgLevel::Default => return,
        BgLevel::Subtle => theme.bg_subtle,
    };
    if color == Color::Reset {
        return;
    }
    frame.render_widget(Block::default().style(Style::default().bg(color)), area);
}

fn draw_panel(frame: &mut Frame, area: Rect, panel: &Option<Panel>, theme: &Theme) -> Rect {
    match panel {
        None => area,
        Some(p) => {
            let block = build_block(p, theme);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            inner
        }
    }
}

fn build_block<'a>(panel: &'a Panel, theme: &Theme) -> Block<'a> {
    let mut b = Block::default()
        .borders(to_borders(panel.border))
        .border_type(to_border_type(panel.border))
        .border_style(Style::default().fg(theme.panel_border));
    if let Some(t) = panel.title.as_deref() {
        b = b
            .title(Span::styled(
                t.to_string(),
                Style::default().fg(theme.panel_title),
            ))
            .title_alignment(to_alignment(panel.title_align));
    }
    b
}

fn to_alignment(align: TitleAlign) -> Alignment {
    match align {
        TitleAlign::Left => Alignment::Left,
        TitleAlign::Center => Alignment::Center,
        TitleAlign::Right => Alignment::Right,
    }
}

fn to_border_type(style: BorderStyle) -> BorderType {
    match style {
        BorderStyle::Plain | BorderStyle::Top => BorderType::Plain,
        BorderStyle::Rounded => BorderType::Rounded,
        BorderStyle::Thick => BorderType::Thick,
        BorderStyle::Double => BorderType::Double,
    }
}

fn to_borders(style: BorderStyle) -> Borders {
    match style {
        BorderStyle::Top => Borders::TOP,
        _ => Borders::ALL,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stack(
    frame: &mut Frame,
    area: Rect,
    direction: Direction,
    flex: Flex,
    children: &[Child],
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
    theme: &Theme,
    general: &General,
    loading: &HashMap<WidgetId, Shape>,
) {
    let axis = direction;
    let constraints: Vec<Constraint> = children
        .iter()
        .map(|c| to_constraint(c.size, &c.layout, axis, area, widgets, specs, registry))
        .collect();
    let rects = RatLayout::default()
        .direction(to_ratatui_direction(direction))
        .constraints(constraints)
        .flex(to_ratatui_flex(flex))
        .split(area);
    for (child, rect) in children.iter().zip(rects.iter()) {
        draw(
            frame,
            *rect,
            &child.layout,
            widgets,
            specs,
            registry,
            theme,
            general,
            loading,
        );
    }
}

fn to_ratatui_flex(flex: Flex) -> RatFlex {
    match flex {
        Flex::Legacy => RatFlex::Legacy,
        Flex::Start => RatFlex::Start,
        Flex::Center => RatFlex::Center,
        Flex::End => RatFlex::End,
        Flex::SpaceBetween => RatFlex::SpaceBetween,
        Flex::SpaceAround => RatFlex::SpaceAround,
    }
}

fn to_constraint(
    size: Size,
    layout: &Layout,
    axis: Direction,
    parent: Rect,
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
) -> Constraint {
    match size {
        Size::Fill(w) => Constraint::Fill(w),
        Size::Length(n) => Constraint::Length(n),
        Size::Min(n) => Constraint::Min(n),
        Size::Max(n) => Constraint::Max(n),
        Size::Percentage(p) => Constraint::Percentage(p),
        Size::Auto => Constraint::Length(natural_length(
            layout, axis, parent, widgets, specs, registry,
        )),
    }
}

/// Natural size along `axis` for a layout subtree. For widget leaves it asks
/// the renderer via `natural_height` (used for both axes today — the only real
/// consumer is `text_ascii` measuring wrapped figlet output, which is vertical
/// only). Stacks recurse: matching-axis stacks sum their children, cross-axis
/// stacks take the max. Panels add 1 to the matching axis for a top border.
fn natural_length(
    layout: &Layout,
    axis: Direction,
    parent: Rect,
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
) -> u16 {
    let size = match layout {
        Layout::Widget { id, panel, .. } => {
            let base = widgets
                .get(id)
                .map(|p| widget_natural(id, &p.body, specs, registry, parent.width))
                .unwrap_or((1, 1));
            let (w, h) = base;
            let axis_value = if matches!(axis, Direction::Vertical) {
                h
            } else {
                w
            };
            axis_value + panel_axis_overhead(panel, axis)
        }
        Layout::Stack {
            direction,
            children,
            panel,
            ..
        } => {
            let values = children.iter().map(|c| match c.size {
                Size::Length(n) | Size::Min(n) | Size::Max(n) if axis == *direction => n,
                _ => natural_length(&c.layout, axis, parent, widgets, specs, registry),
            });
            let combined = if axis == *direction {
                values.sum::<u16>()
            } else {
                values.max().unwrap_or(0)
            };
            combined + panel_axis_overhead(panel, axis)
        }
        Layout::Empty => 0,
    };
    size.max(1)
}

fn panel_axis_overhead(panel: &Option<Panel>, axis: Direction) -> u16 {
    let Some(p) = panel else { return 0 };
    match p.border {
        BorderStyle::Top if matches!(axis, Direction::Vertical) => 1,
        BorderStyle::Plain | BorderStyle::Rounded | BorderStyle::Thick | BorderStyle::Double => 2,
        _ => 0,
    }
}

fn widget_natural(
    id: &str,
    body: &crate::payload::Body,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
    max_width: u16,
) -> (u16, u16) {
    let spec = specs.get(id);
    let name = spec
        .map(|s| s.renderer_name().to_string())
        .unwrap_or_else(|| {
            super::render::default_renderer_for(super::render::shape_of(body)).into()
        });
    let options = spec.map(|s| s.options()).unwrap_or_default();
    let renderer = match registry.get(&name) {
        Some(r) => r,
        None => return (1, 1),
    };
    (
        max_width,
        renderer.natural_height(body, &options, max_width, registry),
    )
}

fn to_ratatui_direction(d: Direction) -> RatDir {
    match d {
        Direction::Vertical => RatDir::Vertical,
        Direction::Horizontal => RatDir::Horizontal,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::payload::{Body, Payload, TextBlockData};
    use crate::render::test_utils::{line_text, render_to_buffer_with};

    fn text_widget(lines: &[&str]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::TextBlock(TextBlockData {
                lines: lines.iter().map(|s| s.to_string()).collect(),
            }),
        }
    }

    fn widgets(pairs: &[(&str, Payload)]) -> HashMap<WidgetId, Payload> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn bare_widget_renders_content_at_top() {
        let l = Layout::widget("g");
        let w = widgets(&[("g", text_widget(&["hello"]))]);
        let buf = render_to_buffer_with(&l, &w, 20, 5);
        assert!(line_text(&buf, 0).contains("hello"));
    }

    #[test]
    fn widget_with_panel_draws_border_and_title() {
        let l = Layout::widget("g").titled("Greeting");
        let w = widgets(&[("g", text_widget(&["hello"]))]);
        let buf = render_to_buffer_with(&l, &w, 20, 5);
        let top = line_text(&buf, 0);
        assert!(top.contains("Greeting"));
        assert!(line_text(&buf, 1).contains("hello"));
    }

    #[test]
    fn stack_with_panel_wraps_children() {
        let l = Layout::cols(vec![
            Child::fill(1, Layout::widget("a")),
            Child::fill(1, Layout::widget("b")),
        ])
        .titled("System");
        let w = widgets(&[("a", text_widget(&["cpu"])), ("b", text_widget(&["mem"]))]);
        let buf = render_to_buffer_with(&l, &w, 40, 5);
        assert!(line_text(&buf, 0).contains("System"));
        let inner = line_text(&buf, 1);
        assert!(inner.contains("cpu"));
        assert!(inner.contains("mem"));
    }

    #[test]
    fn top_border_paints_only_the_top_edge_with_title() {
        let l = Layout::widget("g")
            .titled("branch")
            .bordered(BorderStyle::Top);
        let w = widgets(&[("g", text_widget(&["main"]))]);
        let buf = render_to_buffer_with(&l, &w, 30, 4);
        let top = line_text(&buf, 0);
        // Top rule carries the horizontal glyph and the title hangs off it.
        assert!(top.contains('─'), "missing horizontal rule: {top:?}");
        assert!(top.contains("branch"), "missing title: {top:?}");
        // No corner glyphs — Borders::TOP only draws the top edge.
        assert!(!top.contains('┌') && !top.contains('┐'));
        // Side edges must not render: the second row should not contain left/right glyphs.
        let body = line_text(&buf, 1);
        assert!(!body.contains('│'), "unexpected side chrome: {body:?}");
    }

    #[test]
    fn bordered_style_applies_rounded_corners() {
        let l = Layout::widget("g")
            .titled("T")
            .bordered(BorderStyle::Rounded);
        let w = widgets(&[("g", text_widget(&["x"]))]);
        let buf = render_to_buffer_with(&l, &w, 20, 5);
        assert!(line_text(&buf, 0).contains("╭"));
    }

    #[test]
    fn missing_widget_id_renders_nothing() {
        let l = Layout::widget("missing");
        let w: HashMap<WidgetId, Payload> = HashMap::new();
        let _ = render_to_buffer_with(&l, &w, 20, 5);
    }

    #[test]
    fn empty_layout_renders_nothing() {
        let l = Layout::empty();
        let w: HashMap<WidgetId, Payload> = HashMap::new();
        let _ = render_to_buffer_with(&l, &w, 20, 5);
    }

    #[test]
    fn vertical_stack_splits_area() {
        let l = Layout::rows(vec![
            Child::fill(1, Layout::widget("top")),
            Child::fill(1, Layout::widget("bot")),
        ]);
        let w = widgets(&[
            ("top", text_widget(&["top-content"])),
            ("bot", text_widget(&["bot-content"])),
        ]);
        let buf = render_to_buffer_with(&l, &w, 30, 10);
        let upper: String = (0..5)
            .map(|y| line_text(&buf, y))
            .collect::<Vec<_>>()
            .join(" ");
        let lower: String = (5..10)
            .map(|y| line_text(&buf, y))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(upper.contains("top-content"));
        assert!(lower.contains("bot-content"));
    }

    #[test]
    fn percentage_and_max_constraints_render() {
        let l = Layout::cols(vec![
            Child::percentage(30, Layout::widget("a")),
            Child::max(10, Layout::widget("b")),
            Child::fill(1, Layout::widget("c")),
        ]);
        let w = widgets(&[
            ("a", text_widget(&["a"])),
            ("b", text_widget(&["b"])),
            ("c", text_widget(&["c"])),
        ]);
        let buf = render_to_buffer_with(&l, &w, 40, 5);
        let row = line_text(&buf, 0);
        assert!(row.contains("a"));
        assert!(row.contains("b"));
        assert!(row.contains("c"));
    }

    #[test]
    fn nested_rows_and_cols() {
        let l = Layout::rows(vec![Child::fill(
            1,
            Layout::cols(vec![
                Child::fill(1, Layout::widget("l")),
                Child::fill(1, Layout::widget("r")),
            ]),
        )]);
        let w = widgets(&[
            ("l", text_widget(&["left"])),
            ("r", text_widget(&["right"])),
        ]);
        let buf = render_to_buffer_with(&l, &w, 40, 5);
        let row1 = line_text(&buf, 0);
        assert!(row1.contains("left"));
        assert!(row1.contains("right"));
    }

    #[test]
    fn loading_widget_draws_spinner_instead_of_payload() {
        use ratatui::{Terminal, backend::TestBackend};
        let l = Layout::widget("pending");
        // Payload exists but we also flag the widget as loading — loading takes precedence so
        // the spinner wins over any stale payload body sitting in the map.
        let w = widgets(&[("pending", text_widget(&["stale data"]))]);
        let specs = HashMap::new();
        let registry = Registry::with_builtins();
        let theme = Theme::default();
        let mut loading: HashMap<WidgetId, Shape> = HashMap::new();
        loading.insert("pending".into(), Shape::Entries);
        let backend = TestBackend::new(30, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    f.area(),
                    &l,
                    &w,
                    &specs,
                    &registry,
                    &theme,
                    &General::default(),
                    &loading,
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let dump: String = (0..buf.area().height).map(|y| line_text(&buf, y)).collect();
        assert!(
            dump.contains("loading"),
            "expected loading spinner label, got:\n{dump}"
        );
        assert!(
            !dump.contains("stale data"),
            "loading should suppress payload content, got:\n{dump}"
        );
    }
}
