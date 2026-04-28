//! Convert a ratatui `Buffer` into an inline HTML fragment for embedding in Markdown pages.
//!
//! Output: a single `<pre class="splash-snapshot">` with one `<span>` per contiguous run of
//! cells that share fg/bg/modifier. Default-styled cells emit plain text (no wrapping span) so
//! the rendered markup stays readable and small.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

pub fn buffer_to_html(buf: &Buffer) -> String {
    let mut out = String::new();
    out.push_str("<pre class=\"splash-snapshot\">");
    let area = buf.area();
    for y in 0..area.height {
        emit_row(&mut out, buf, y, area.width);
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

fn emit_row(out: &mut String, buf: &Buffer, y: u16, width: u16) {
    let mut run_style: Option<Style> = None;
    let mut run_cells: Vec<String> = Vec::new();
    let mut x = 0u16;
    while x < width {
        let cell = buf[(x, y)].clone();
        // Cells flagged `skip` are placeholders for the visible region of an OSC 8 hyperlink
        // — the link's first cell carries the whole sequence; the trailing cells exist only
        // so terminal cursor math lines up. Drop them in HTML to avoid emitting stray spaces.
        if cell.skip {
            x += 1;
            continue;
        }
        let style = cell_style(&cell);
        if let Some((url, visible)) = parse_osc8(cell.symbol()) {
            flush_run(out, run_style.as_ref(), &run_cells);
            run_cells.clear();
            run_style = None;
            emit_link(out, &style, &url, &visible);
            // The TestBackend stores the OSC 8 sequence in the first cell only; the trailing
            // cells the renderer flagged with `skip` aren't preserved across the diff/flush
            // (the backend's per-cell store has no `skip` field), so they keep whatever the
            // prior frame left there. Step past them ourselves using the visible text's
            // display width so we don't double-emit the underlying spaces / glyphs.
            let cells_consumed = UnicodeWidthStr::width(visible.as_str()).max(1) as u16;
            x = x.saturating_add(cells_consumed);
            continue;
        }
        if Some(style) != run_style {
            flush_run(out, run_style.as_ref(), &run_cells);
            run_cells.clear();
            run_style = Some(style);
        }
        run_cells.push(cell.symbol().to_string());
        x += 1;
    }
    flush_run(out, run_style.as_ref(), &run_cells);
}

/// Parse an OSC 8 hyperlink wrapper of the form `ESC ] 8 ; ; <url> ESC \ <visible> ESC ] 8 ; ; ESC \`
/// into its URL and visible-text components. The terminal renderer (`list_links`) embeds the
/// whole sequence into a single cell symbol; in HTML we need the parts so we can emit a real
/// `<a>` tag instead of leaking the escape codes as text.
fn parse_osc8(symbol: &str) -> Option<(String, String)> {
    const PREFIX: &str = "\x1b]8;;";
    const ST: &str = "\x1b\\";
    const CLOSE: &str = "\x1b]8;;\x1b\\";
    let s = symbol.strip_prefix(PREFIX)?;
    let url_end = s.find(ST)?;
    let url = &s[..url_end];
    let after_url = &s[url_end + ST.len()..];
    let close_pos = after_url.rfind(CLOSE)?;
    let visible = &after_url[..close_pos];
    Some((url.to_string(), visible.to_string()))
}

fn emit_link(out: &mut String, style: &Style, url: &str, visible: &str) {
    let styled = has_visible_style(style);
    if styled {
        out.push_str("<span style=\"");
        out.push_str(&style_css(style));
        out.push_str("\">");
    }
    out.push_str("<a href=\"");
    out.push_str(&escape_html(url));
    out.push_str("\" class=\"splash-link\">");
    for ch in visible.chars() {
        out.push_str("<span class=\"c\">");
        out.push_str(&escape_html(&ch.to_string()));
        out.push_str("</span>");
    }
    out.push_str("</a>");
    if styled {
        out.push_str("</span>");
    }
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

    #[test]
    fn osc8_hyperlink_becomes_anchor() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 1));
        buf[(0, 0)].set_symbol("\x1b]8;;https://example.com/\x1b\\Hi\x1b]8;;\x1b\\");
        buf[(1, 0)].set_skip(true);
        let html = buffer_to_html(&buf);
        assert!(
            html.contains("<a href=\"https://example.com/\""),
            "expected anchor tag, got: {html}"
        );
        assert!(html.contains("<span class=\"c\">H</span>"));
        assert!(html.contains("<span class=\"c\">i</span>"));
        assert!(!html.contains("\x1b"));
        assert!(!html.contains("]8;;"));
    }

    #[test]
    fn osc8_link_consumes_underlying_cells_even_when_skip_is_lost() {
        // The TestBackend doesn't carry the renderer's `skip` flag across the diff/flush, so
        // the cells trailing an OSC 8 link land back in the buffer as plain spaces. Without
        // the visible-width step in `emit_row`, those would emit as extra cell spans on top
        // of the link's own char spans — a 5-char link in a 10-wide row would produce 5 + 4
        // = 9 spans inside the link area, blowing past the row width.
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        buf[(0, 0)].set_symbol("\x1b]8;;https://example.com/\x1b\\hello\x1b]8;;\x1b\\");
        // No skip flags — emulating what the TestBackend hands back after the diff/flush.
        let html = buffer_to_html(&buf);
        // 5 link chars + 5 trailing cells = 10 cell spans, matching the buffer width.
        assert_eq!(html.matches("<span class=\"c\">").count(), 10);
    }

    #[test]
    fn skip_cells_are_dropped() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf[(0, 0)].set_symbol("a");
        buf[(1, 0)].set_skip(true);
        buf[(2, 0)].set_skip(true);
        buf[(3, 0)].set_symbol("b");
        let html = buffer_to_html(&buf);
        assert_eq!(html.matches("<span class=\"c\">").count(), 2);
    }
}
