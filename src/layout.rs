use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Direction as RatDir, Layout as RatLayout, Rect},
    widgets::{Block, BorderType, Borders},
};

use crate::payload::Payload;
use crate::render::{Registry, RenderSpec, render_payload};

pub type WidgetId = String;

#[derive(Debug, Clone)]
pub enum Layout {
    Stack {
        direction: Direction,
        children: Vec<Child>,
        panel: Option<Panel>,
    },
    Widget {
        id: WidgetId,
        panel: Option<Panel>,
    },
    #[allow(dead_code)]
    Empty,
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
}

#[derive(Debug, Clone, Default)]
pub struct Panel {
    pub title: Option<String>,
    pub border: BorderStyle,
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
}

impl Layout {
    pub fn rows(children: Vec<Child>) -> Self {
        Self::Stack {
            direction: Direction::Vertical,
            children,
            panel: None,
        }
    }

    pub fn cols(children: Vec<Child>) -> Self {
        Self::Stack {
            direction: Direction::Horizontal,
            children,
            panel: None,
        }
    }

    pub fn widget(id: impl Into<WidgetId>) -> Self {
        Self::Widget {
            id: id.into(),
            panel: None,
        }
    }

    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self::Empty
    }

    pub fn titled(self, title: impl Into<String>) -> Self {
        self.with_panel(|p| p.title(title))
    }

    #[allow(dead_code)]
    pub fn bordered(self, border: BorderStyle) -> Self {
        self.with_panel(|p| p.border(border))
    }

    fn with_panel(self, f: impl FnOnce(Panel) -> Panel) -> Self {
        match self {
            Self::Stack {
                direction,
                children,
                panel,
            } => {
                let p = f(panel.unwrap_or_default());
                Self::Stack {
                    direction,
                    children,
                    panel: Some(p),
                }
            }
            Self::Widget { id, panel } => {
                let p = f(panel.unwrap_or_default());
                Self::Widget { id, panel: Some(p) }
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

    #[allow(dead_code)]
    pub fn percentage(percent: u16, layout: Layout) -> Self {
        Self {
            size: Size::Percentage(percent),
            layout,
        }
    }
}

pub fn draw(
    frame: &mut Frame,
    area: Rect,
    layout: &Layout,
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
) {
    match layout {
        Layout::Stack {
            direction,
            children,
            panel,
        } => {
            let inner = draw_panel(frame, area, panel);
            draw_stack(frame, inner, *direction, children, widgets, specs, registry);
        }
        Layout::Widget { id, panel } => {
            let inner = draw_panel(frame, area, panel);
            if let Some(payload) = widgets.get(id) {
                render_payload(frame, inner, payload, specs.get(id), registry);
            }
        }
        Layout::Empty => {}
    }
}

fn draw_panel(frame: &mut Frame, area: Rect, panel: &Option<Panel>) -> Rect {
    match panel {
        None => area,
        Some(p) => {
            let block = build_block(p);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            inner
        }
    }
}

fn build_block(panel: &Panel) -> Block<'_> {
    let mut b = Block::default()
        .borders(Borders::ALL)
        .border_type(to_border_type(panel.border));
    if let Some(t) = panel.title.as_deref() {
        b = b.title(t.to_string());
    }
    b
}

fn to_border_type(style: BorderStyle) -> BorderType {
    match style {
        BorderStyle::Plain => BorderType::Plain,
        BorderStyle::Rounded => BorderType::Rounded,
        BorderStyle::Thick => BorderType::Thick,
        BorderStyle::Double => BorderType::Double,
    }
}

fn draw_stack(
    frame: &mut Frame,
    area: Rect,
    direction: Direction,
    children: &[Child],
    widgets: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &Registry,
) {
    let constraints: Vec<Constraint> = children.iter().map(|c| to_constraint(c.size)).collect();
    let rects = RatLayout::default()
        .direction(to_ratatui_direction(direction))
        .constraints(constraints)
        .split(area);
    for (child, rect) in children.iter().zip(rects.iter()) {
        draw(frame, *rect, &child.layout, widgets, specs, registry);
    }
}

fn to_constraint(size: Size) -> Constraint {
    match size {
        Size::Fill(w) => Constraint::Fill(w),
        Size::Length(n) => Constraint::Length(n),
        Size::Min(n) => Constraint::Min(n),
        Size::Max(n) => Constraint::Max(n),
        Size::Percentage(p) => Constraint::Percentage(p),
    }
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
    use crate::payload::{Body, LinesData, Payload};
    use crate::render::test_utils::{line_text, render_to_buffer_with};

    fn text_widget(lines: &[&str]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData {
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
}
