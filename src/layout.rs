use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Constraint, Direction as RatDir, Layout as RatLayout, Rect},
};

use crate::payload::Payload;
use crate::render::render_payload;

pub type WidgetId = String;

#[derive(Debug, Clone)]
pub enum Layout {
    Stack {
        direction: Direction,
        children: Vec<Child>,
    },
    Widget(WidgetId),
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
}

impl Layout {
    pub fn rows(children: Vec<Child>) -> Self {
        Self::Stack {
            direction: Direction::Vertical,
            children,
        }
    }

    pub fn cols(children: Vec<Child>) -> Self {
        Self::Stack {
            direction: Direction::Horizontal,
            children,
        }
    }

    pub fn widget(id: impl Into<WidgetId>) -> Self {
        Self::Widget(id.into())
    }

    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self::Empty
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
}

pub fn draw(frame: &mut Frame, area: Rect, layout: &Layout, widgets: &HashMap<WidgetId, Payload>) {
    match layout {
        Layout::Stack {
            direction,
            children,
        } => draw_stack(frame, area, *direction, children, widgets),
        Layout::Widget(id) => {
            if let Some(payload) = widgets.get(id) {
                render_payload(frame, area, payload);
            }
        }
        Layout::Empty => {}
    }
}

fn draw_stack(
    frame: &mut Frame,
    area: Rect,
    direction: Direction,
    children: &[Child],
    widgets: &HashMap<WidgetId, Payload>,
) {
    let constraints: Vec<Constraint> = children.iter().map(|c| to_constraint(c.size)).collect();
    let rects = RatLayout::default()
        .direction(to_ratatui_direction(direction))
        .constraints(constraints)
        .split(area);
    for (child, rect) in children.iter().zip(rects.iter()) {
        draw(frame, *rect, &child.layout, widgets);
    }
}

fn to_constraint(size: Size) -> Constraint {
    match size {
        Size::Fill(w) => Constraint::Fill(w),
        Size::Length(n) => Constraint::Length(n),
        Size::Min(n) => Constraint::Min(n),
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
    use crate::payload::{Body, Payload, TextData};
    use crate::render::test_utils::{line_text, render_to_buffer_with};

    fn text_widget(lines: &[&str]) -> Payload {
        Payload {
            title: None,
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
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
    fn widget_leaf_renders() {
        let l = Layout::widget("g");
        let w = widgets(&[("g", text_widget(&["hello"]))]);
        let buf = render_to_buffer_with(&l, &w, 20, 5);
        assert!(line_text(&buf, 1).contains("hello"));
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
        let row1 = line_text(&buf, 1);
        assert!(row1.contains("left"));
        assert!(row1.contains("right"));
    }
}
