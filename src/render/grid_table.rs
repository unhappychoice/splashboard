use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    pub layout: Option<String>,
    #[serde(default)]
    pub column_align: Option<Vec<String>>,
}

const COLOR_KEYS: &[ColorKey] = &[
    theme::STATUS_OK,
    theme::STATUS_WARN,
    theme::STATUS_ERROR,
    theme::TEXT,
    theme::TEXT_DIM,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "layout",
        type_hint: "\"rows\" | \"inline\"",
        required: false,
        default: Some("\"rows\""),
        description: "Layout mode. `rows` draws one key/value row per entry; `inline` condenses to a single line of `key: value · key: value`.",
    },
    OptionSchema {
        name: "column_align",
        type_hint: "array of \"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("[\"left\", \"left\"]"),
        description: "Per-column alignment. First entry aligns the key column, second the value column. Ignored in `inline` layout.",
    },
    OptionSchema {
        name: "align",
        type_hint: "\"left\" | \"center\" | \"right\"",
        required: false,
        default: Some("\"left\""),
        description: "Horizontal alignment of the rendered line. Honoured in `inline` layout only — `rows` uses `column_align` per column instead.",
    },
];

/// Key/value table renderer over the `Entries` shape. Internally a ratatui `Table` with two
/// columns (key, value) — the name reflects the widget, not the data, so future alternative
/// renderers for the same shape (card layout, inline chips) don't have to fight over "list".
pub struct GridTableRenderer;

impl Renderer for GridTableRenderer {
    fn name(&self) -> &str {
        "grid_table"
    }
    fn description(&self) -> &'static str {
        "Two-column key/value rows tinted by per-row status, with an `inline` layout that condenses the same entries into a single `key: value · key: value` line. The default look for entry-style payloads (system info, project metadata, env summaries)."
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Entries]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn render(
        &self,
        frame: &mut Frame,
        area: Rect,
        body: &Body,
        opts: &RenderOptions,
        theme: &Theme,
        _registry: &Registry,
    ) {
        if let Body::Entries(d) = body {
            let specific: Options = opts.parse_specific();
            match specific.layout.as_deref() {
                Some("inline") => render_inline(frame, area, d, opts, theme),
                _ => render_rows(frame, area, d, &specific, theme),
            }
        }
    }
}

fn render_rows(
    frame: &mut Frame,
    area: Rect,
    data: &EntriesData,
    specific: &Options,
    theme: &Theme,
) {
    let column_align = column_alignments(specific.column_align.as_deref());
    let rows = data.items.iter().map(|e| to_row(e, column_align, theme));
    let widths = [Constraint::Percentage(40), Constraint::Percentage(60)];
    frame.render_widget(
        Table::new(rows, widths).style(Style::default().fg(theme.text)),
        area,
    );
}

fn render_inline(
    frame: &mut Frame,
    area: Rect,
    data: &EntriesData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    let text = Style::default().fg(theme.text);
    let dim = Style::default().fg(theme.text_dim);
    let mut spans: Vec<Span> = Vec::new();
    for (i, entry) in data.items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", dim));
        }
        let row_style = entry
            .status
            .map(|s| Style::default().fg(super::status_badge::status_color(s, theme)))
            .unwrap_or(text);
        spans.push(Span::styled(entry.key.clone(), row_style));
        if let Some(v) = entry.value.as_deref() {
            spans.push(Span::styled(": ", row_style));
            spans.push(Span::styled(v.to_string(), row_style));
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans))
            .style(text)
            .alignment(parse_align(opts.align.as_deref())),
        area,
    );
}

fn parse_align(s: Option<&str>) -> Alignment {
    match s {
        Some("center") => Alignment::Center,
        Some("right") => Alignment::Right,
        _ => Alignment::Left,
    }
}

fn column_alignments(raw: Option<&[String]>) -> [Alignment; 2] {
    let get = |i: usize| -> Alignment {
        raw.and_then(|a| a.get(i))
            .map(|s| match s.as_str() {
                "center" => Alignment::Center,
                "right" => Alignment::Right,
                _ => Alignment::Left,
            })
            .unwrap_or(Alignment::Left)
    };
    [get(0), get(1)]
}

fn to_row<'a>(item: &'a Entry, aligns: [Alignment; 2], theme: &Theme) -> Row<'a> {
    let key = Cell::from(Line::from(item.key.clone()).alignment(aligns[0]));
    let value = Cell::from(Line::from(item.value.clone().unwrap_or_default()).alignment(aligns[1]));
    let mut row = Row::new(vec![key, value]);
    if let Some(status) = item.status {
        row = row.style(Style::default().fg(super::status_badge::status_color(status, theme)));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{EntriesData, Entry, Payload, Status};
    use crate::render::test_utils::{line_text, render_to_buffer, render_to_buffer_with_spec};
    use crate::render::{Registry, RenderSpec};

    fn payload(items: Vec<Entry>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Entries(EntriesData { items }),
        }
    }

    #[test]
    fn renders_key_and_value() {
        let p = payload(vec![Entry {
            key: "uptime".into(),
            value: Some("3d".into()),
            status: Some(Status::Ok),
        }]);
        let buf = render_to_buffer(&p, 30, 5);
        let row = line_text(&buf, 0);
        assert!(row.contains("uptime"));
        assert!(row.contains("3d"));
    }

    #[test]
    fn inline_layout_joins_entries_on_single_line() {
        let registry = Registry::with_builtins();
        #[derive(serde::Deserialize)]
        struct W {
            render: RenderSpec,
        }
        let w: W =
            toml::from_str(r#"render = { type = "grid_table", layout = "inline" }"#).unwrap();
        let p = payload(vec![
            Entry {
                key: "stars".into(),
                value: Some("1.2k".into()),
                status: None,
            },
            Entry {
                key: "license".into(),
                value: Some("MIT".into()),
                status: None,
            },
        ]);
        let buf = render_to_buffer_with_spec(&p, Some(&w.render), &registry, 60, 1);
        let row = line_text(&buf, 0);
        assert!(row.contains("stars: 1.2k"), "row: {row:?}");
        assert!(row.contains("license: MIT"), "row: {row:?}");
        assert!(row.contains('·'), "separator missing: {row:?}");
    }
}
