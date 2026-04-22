//! Convert a ratatui `Buffer` into an inline HTML fragment for embedding in Markdown pages.
//!
//! Output: a single `<pre class="splash-snapshot">` with one `<span>` per contiguous run of
//! cells that share fg/bg/modifier. Default-styled cells emit plain text (no wrapping span) so
//! the rendered markup stays readable and small.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

pub fn buffer_to_html(buf: &Buffer) -> String {
    let mut out = String::new();
    out.push_str("<pre class=\"splash-snapshot\">");
    let area = buf.area();
    for y in 0..area.height {
        let mut run_style: Option<Style> = None;
        let mut run_cells: Vec<String> = Vec::new();
        for x in 0..area.width {
            let cell = buf[(x, y)].clone();
            let style = cell_style(&cell);
            if Some(style) != run_style {
                flush_run(&mut out, run_style.as_ref(), &run_cells);
                run_cells.clear();
                run_style = Some(style);
            }
            run_cells.push(cell.symbol().to_string());
        }
        flush_run(&mut out, run_style.as_ref(), &run_cells);
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

/// Skip `Color::Reset` (ratatui's "no explicit color" sentinel) so default cells produce plain
/// text rather than gratuitous `<span>` tags covering the whole buffer.
fn cell_style(cell: &ratatui::buffer::Cell) -> Style {
    let mut s = Style::default().add_modifier(cell.modifier);
    if cell.fg != Color::Reset {
        s = s.fg(cell.fg);
    }
    if cell.bg != Color::Reset {
        s = s.bg(cell.bg);
    }
    s
}

/// Emit one cell per `<span class="c">` so the browser lays out every terminal cell into a
/// fixed 1ch-wide box regardless of how the font renders the individual glyph. Without this,
/// block characters (▟▜▛ etc.) get slightly different advance widths per-glyph and column
/// alignment drifts. Runs of same-styled cells share an outer styled span.
fn flush_run(out: &mut String, style: Option<&Style>, cells: &[String]) {
    if cells.is_empty() {
        return;
    }
    match style {
        Some(s) if has_visible_style(s) => {
            out.push_str("<span style=\"");
            out.push_str(&style_css(s));
            out.push_str("\">");
            emit_cells(out, cells);
            out.push_str("</span>");
        }
        _ => emit_cells(out, cells),
    }
}

fn emit_cells(out: &mut String, cells: &[String]) {
    for cell in cells {
        out.push_str("<span class=\"c\">");
        out.push_str(&escape_html(cell));
        out.push_str("</span>");
    }
}

fn has_visible_style(s: &Style) -> bool {
    s.fg.is_some() || s.bg.is_some() || !s.add_modifier.is_empty()
}

fn style_css(s: &Style) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = s.fg {
        parts.push(format!("color:{}", color_hex(c)));
    }
    if let Some(c) = s.bg {
        parts.push(format!("background:{}", color_hex(c)));
    }
    if s.add_modifier.contains(Modifier::BOLD) {
        parts.push("font-weight:bold".into());
    }
    if s.add_modifier.contains(Modifier::ITALIC) {
        parts.push("font-style:italic".into());
    }
    if s.add_modifier.contains(Modifier::UNDERLINED) {
        parts.push("text-decoration:underline".into());
    }
    if s.add_modifier.contains(Modifier::DIM) {
        parts.push("opacity:0.6".into());
    }
    parts.join(";")
}

/// ANSI-16 baseline; truecolor passes through. `Reset`/`Indexed` fall back to `currentColor`
/// so dark-mode CSS can flip the page foreground without the snapshots going black-on-black.
fn color_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Black => "#000000".into(),
        Color::Red => "#aa0000".into(),
        Color::Green => "#00aa00".into(),
        Color::Yellow => "#aa5500".into(),
        Color::Blue => "#0000aa".into(),
        Color::Magenta => "#aa00aa".into(),
        Color::Cyan => "#00aaaa".into(),
        Color::Gray => "#aaaaaa".into(),
        Color::DarkGray => "#555555".into(),
        Color::LightRed => "#ff5555".into(),
        Color::LightGreen => "#55ff55".into(),
        Color::LightYellow => "#ffff55".into(),
        Color::LightBlue => "#5555ff".into(),
        Color::LightMagenta => "#ff55ff".into(),
        Color::LightCyan => "#55ffff".into(),
        Color::White => "#ffffff".into(),
        Color::Reset | Color::Indexed(_) => "currentColor".into(),
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn buffer_wraps_every_cell_in_its_own_span() {
        let buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        let html = buffer_to_html(&buf);
        assert!(html.starts_with("<pre"));
        assert!(html.ends_with("</pre>"));
        // 3 cols × 2 rows = 6 cells → 6 per-cell spans.
        assert_eq!(html.matches("<span class=\"c\">").count(), 6);
    }

    #[test]
    fn escapes_html_metacharacters() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        buf[(0, 0)].set_symbol("<");
        buf[(1, 0)].set_symbol("&");
        buf[(2, 0)].set_symbol(">");
        let html = buffer_to_html(&buf);
        assert!(html.contains("&lt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&gt;"));
    }
}
