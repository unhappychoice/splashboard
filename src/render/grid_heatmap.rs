use ratatui::{Frame, buffer::Buffer, layout::Rect};

use crate::payload::{Body, HeatmapData};
use crate::theme::{self, ColorKey, Theme};

use super::{Registry, RenderOptions, Renderer, Shape};

const COLOR_KEYS: &[ColorKey] = &[theme::PALETTE_HEATMAP, theme::TEXT];

/// 2D intensity grid — GitHub-style contribution graph and any other daily/periodic heatmap.
/// Each cell takes two terminal columns and one row so the grid reads as "dots of equal width"
/// despite the 1:2 character aspect ratio. Buckets each cell's value into one of 5 intensity
/// levels; level 0 is dim background, level 4 is full-saturation theme color. Thresholds come
/// from the payload when provided, otherwise fall back to an auto-quartile split of the data.
pub struct GridHeatmapRenderer;

const LEVELS: usize = 5;
/// Each cell takes 2 terminal columns so it reads as square-ish despite the 1:2 character
/// aspect. The first column paints the filled glyph; the second is a blank spacer so adjacent
/// cells don't merge into a single band. 1 row per cell — rows are already tall.
const CELL_W: u16 = 2;
const CELL_H: u16 = 1;
const CELL_GLYPH: &str = "■";

impl Renderer for GridHeatmapRenderer {
    fn name(&self) -> &str {
        "grid_heatmap"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Heatmap]
    }
    fn color_keys(&self) -> &[ColorKey] {
        COLOR_KEYS
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
        if let Body::Heatmap(d) = body {
            render_heatmap(frame, area, d, opts, theme);
        }
    }
}

fn render_heatmap(
    frame: &mut Frame,
    area: Rect,
    data: &HeatmapData,
    opts: &RenderOptions,
    theme: &Theme,
) {
    if data.cells.is_empty() || data.cells[0].is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let thresholds = resolve_thresholds(data);
    // Reserve a top row for column labels only when we have labels AND enough vertical room
    // to draw them without eating into the actual grid.
    let label_row = has_col_labels(data) && area.height as usize > data.cells.len();
    let grid_area = if label_row {
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        }
    } else {
        area
    };
    let (visible_cols, col_offset) = visible_col_window(data, grid_area);
    // Horizontal alignment: the grid itself is narrower than the slot when the terminal is
    // wide. Without alignment the grid hugs the left edge, which looks lopsided next to
    // centered widgets above/below it. `align` (left / center / right) shifts the grid's
    // x-origin inside `area`.
    let grid_px_width = (visible_cols as u16) * CELL_W;
    let x_shift = horizontal_shift(opts.align.as_deref(), grid_area.width, grid_px_width);
    let shifted = Rect {
        x: grid_area.x + x_shift,
        ..grid_area
    };
    let label_area = Rect {
        x: area.x + x_shift,
        ..area
    };
    if label_row {
        paint_col_labels(
            frame.buffer_mut(),
            label_area,
            data,
            visible_cols,
            col_offset,
            theme,
        );
    }
    paint_cells(
        frame.buffer_mut(),
        shifted,
        data,
        &thresholds,
        visible_cols,
        col_offset,
        theme,
    );
}

fn horizontal_shift(align: Option<&str>, area_width: u16, content_width: u16) -> u16 {
    if content_width >= area_width {
        return 0;
    }
    let slack = area_width - content_width;
    match align {
        Some("center") => slack / 2,
        Some("right") => slack,
        _ => 0,
    }
}

fn has_col_labels(data: &HeatmapData) -> bool {
    data.col_labels
        .as_ref()
        .is_some_and(|v| v.iter().any(|s| !s.is_empty()))
}

fn visible_col_window(data: &HeatmapData, grid_area: Rect) -> (usize, usize) {
    let cols = data.cells[0].len();
    let max_visible_cols = (grid_area.width / CELL_W) as usize;
    let visible_cols = cols.min(max_visible_cols);
    // Right-align columns so "the most recent N weeks" is what survives when the slot is
    // narrower than the full year. Row alignment is top-anchored (weekday order is fixed).
    let col_offset = cols.saturating_sub(visible_cols);
    (visible_cols, col_offset)
}

#[allow(clippy::too_many_arguments)]
fn paint_cells(
    buf: &mut Buffer,
    area: Rect,
    data: &HeatmapData,
    thresholds: &[u32],
    visible_cols: usize,
    col_offset: usize,
    theme: &Theme,
) {
    let rows = data.cells.len();
    let max_visible_rows = (area.height / CELL_H) as usize;
    let visible_rows = rows.min(max_visible_rows);
    for r in 0..visible_rows {
        for c in 0..visible_cols {
            let value = data.cells[r][c + col_offset];
            let level = bucket(value, thresholds);
            let color = theme.heatmap_level(level);
            let x = area.x + (c as u16) * CELL_W;
            let y = area.y + (r as u16) * CELL_H;
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(CELL_GLYPH);
                cell.set_fg(color);
            }
            // Column 1 of each cell stays as the terminal default so the grid reads as
            // distinct cells rather than a single continuous band.
        }
    }
}

fn paint_col_labels(
    buf: &mut Buffer,
    area: Rect,
    data: &HeatmapData,
    visible_cols: usize,
    col_offset: usize,
    theme: &Theme,
) {
    // Labels live on the top row of `area` (which includes the label row; `grid_area` starts
    // one row below). Write each non-empty label at its column's x-origin, extending as far
    // as there's room before the next label starts.
    let Some(labels) = &data.col_labels else {
        return;
    };
    let mut next_label_start: usize = visible_cols;
    // Scan from right to left so we know where the next label starts when we paint the
    // current one — lets us stop writing before colliding with the neighbor.
    for c in (0..visible_cols).rev() {
        let Some(label) = labels.get(c + col_offset) else {
            continue;
        };
        if label.is_empty() {
            continue;
        }
        let x0 = area.x + (c as u16) * CELL_W;
        let max_chars = (next_label_start - c) * CELL_W as usize;
        for (i, ch) in label.chars().take(max_chars).enumerate() {
            if let Some(cell) = buf.cell_mut((x0 + i as u16, area.y)) {
                cell.set_symbol(&ch.to_string());
                cell.set_fg(theme.text);
            }
        }
        next_label_start = c;
    }
}

fn bucket(value: u32, thresholds: &[u32]) -> usize {
    // thresholds has exactly 4 boundaries: level 0 = value == 0; level 1..=4 = above each
    // boundary. Keeping level 0 reserved for "no activity" matches the GitHub convention and
    // reads as visually distinct even when everything else is faint.
    if value == 0 {
        return 0;
    }
    let mut level = 1;
    for (i, &boundary) in thresholds.iter().enumerate() {
        if value >= boundary {
            level = (i + 1).min(LEVELS - 1) + 1;
        }
    }
    level.min(LEVELS - 1)
}

fn resolve_thresholds(data: &HeatmapData) -> Vec<u32> {
    if let Some(t) = &data.thresholds
        && !t.is_empty()
    {
        // User-provided: take up to 4 ascending boundaries; pad/truncate for deterministic
        // bucket count.
        let mut t = t.clone();
        t.sort_unstable();
        t.truncate(LEVELS - 1);
        while t.len() < LEVELS - 1 {
            t.push(t.last().copied().unwrap_or(1).saturating_add(1));
        }
        return t;
    }
    auto_quartile_thresholds(data)
}

fn auto_quartile_thresholds(data: &HeatmapData) -> Vec<u32> {
    let mut nonzero: Vec<u32> = data
        .cells
        .iter()
        .flat_map(|r| r.iter().copied())
        .filter(|v| *v > 0)
        .collect();
    if nonzero.is_empty() {
        return vec![1, 2, 3, 4];
    }
    nonzero.sort_unstable();
    let n = nonzero.len();
    // Four evenly-spaced percentiles (roughly 25/50/75/90) for the 4 boundaries between
    // levels 1-4. Using >75% as the top boundary keeps rare spikes visually distinct
    // without letting outliers dominate.
    let pick = |pct: usize| nonzero[((n * pct) / 100).min(n - 1)];
    let mut out = vec![pick(25), pick(50), pick(75), pick(90)];
    // Ensure strictly increasing so buckets are well-defined even on low-variance data.
    for i in 1..out.len() {
        if out[i] <= out[i - 1] {
            out[i] = out[i - 1] + 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{HeatmapData, Payload};
    use crate::render::test_utils::render_to_buffer_with_spec;
    use crate::render::{Registry, RenderSpec};

    fn payload(cells: Vec<Vec<u32>>) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Heatmap(HeatmapData {
                cells,
                thresholds: None,
                row_labels: None,
                col_labels: None,
            }),
        }
    }

    fn render(cells: Vec<Vec<u32>>, w: u16, h: u16) -> ratatui::buffer::Buffer {
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("grid_heatmap".into());
        render_to_buffer_with_spec(&payload(cells), Some(&spec), &registry, w, h)
    }

    #[test]
    fn empty_cells_no_panic() {
        let _ = render(vec![], 20, 7);
        let _ = render(vec![vec![]], 20, 7);
    }

    #[test]
    fn renders_grid_without_panicking() {
        let cells: Vec<Vec<u32>> = (0..7)
            .map(|r| (0..20).map(|c| (r * c) as u32).collect())
            .collect();
        let _ = render(cells, 40, 7);
    }

    #[test]
    fn zero_always_gets_bucket_zero() {
        let thresholds = vec![1, 2, 3, 4];
        assert_eq!(bucket(0, &thresholds), 0);
    }

    #[test]
    fn nonzero_never_gets_bucket_zero() {
        let thresholds = vec![10, 20, 30, 40];
        // Even a tiny value above 0 must be bucket >= 1 so "has activity" is visually distinct.
        assert!(bucket(1, &thresholds) >= 1);
    }

    #[test]
    fn higher_values_get_higher_buckets() {
        let thresholds = vec![5, 10, 15, 20];
        let low = bucket(3, &thresholds);
        let high = bucket(30, &thresholds);
        assert!(high > low);
    }

    #[test]
    fn auto_quartile_is_strictly_increasing() {
        let cells = vec![vec![0, 1, 1, 1, 1, 2, 3, 5, 8, 13]];
        let data = HeatmapData {
            cells,
            thresholds: None,
            row_labels: None,
            col_labels: None,
        };
        let t = auto_quartile_thresholds(&data);
        for i in 1..t.len() {
            assert!(t[i] > t[i - 1], "thresholds not strictly increasing: {t:?}");
        }
    }

    #[test]
    fn auto_quartile_falls_back_on_all_zero() {
        let data = HeatmapData {
            cells: vec![vec![0, 0, 0]],
            thresholds: None,
            row_labels: None,
            col_labels: None,
        };
        assert_eq!(auto_quartile_thresholds(&data), vec![1, 2, 3, 4]);
    }

    #[test]
    fn right_aligns_wide_grid_in_narrow_area() {
        // 30-column grid, 20 visible columns → should show the rightmost 20.
        let mut cells = vec![vec![0u32; 30]];
        cells[0][29] = 100; // Marker in the last column.
        let buf = render(cells, 40, 1); // 40 cols / 2 per cell = 20 visible columns.
        // The rightmost filled cell (position 38) should carry the filled glyph.
        let right = buf.cell((38, 0)).unwrap();
        assert_eq!(right.symbol(), CELL_GLYPH);
        // The spacer column right after it (position 39) must stay blank.
        let gap = buf.cell((39, 0)).unwrap();
        assert_eq!(gap.symbol(), " ", "spacer column must be blank");
    }

    #[test]
    fn col_labels_paint_on_top_row_when_height_allows() {
        let cells = vec![vec![1u32; 10]; 7];
        let mut labels = vec![String::new(); 10];
        labels[0] = "Jan".into();
        labels[4] = "Feb".into();
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Heatmap(HeatmapData {
                cells,
                thresholds: None,
                row_labels: None,
                col_labels: Some(labels),
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("grid_heatmap".into());
        // 10 cells × 2 cols = 20 width; 7 day rows + 1 label row = 8 height.
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 20, 8);
        let top: String = (0..20)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(top.contains("Jan"), "Jan label missing: {top:?}");
        assert!(top.contains("Feb"), "Feb label missing: {top:?}");
    }

    #[test]
    fn col_labels_skipped_when_height_too_tight() {
        // 7 rows is exactly the grid; no room for a label row, so the grid still paints all
        // 7 day rows with no label collision.
        let cells = vec![vec![5u32; 10]; 7];
        let labels: Vec<String> = (0..10).map(|_| "Jan".into()).collect();
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Heatmap(HeatmapData {
                cells,
                thresholds: None,
                row_labels: None,
                col_labels: Some(labels),
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("grid_heatmap".into());
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 20, 7);
        // The top row should be cells, not a label — first char is the filled glyph.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), CELL_GLYPH);
    }

    #[test]
    fn align_center_shifts_grid_to_middle_of_area() {
        // 5 cells × 2 cols = 10px; area is 20 wide → slack 10 → center pad 5 on the left.
        let cells = vec![vec![5u32; 5]];
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Heatmap(HeatmapData {
                cells,
                thresholds: None,
                row_labels: None,
                col_labels: None,
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Full {
            type_name: "grid_heatmap".into(),
            options: RenderOptions {
                align: Some("center".into()),
                ..Default::default()
            },
        };
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 20, 1);
        // First 5 columns are pre-pad (default ' ').
        assert_eq!(buf.cell((4, 0)).unwrap().symbol(), " ");
        // Column 5 has the first painted glyph.
        assert_eq!(buf.cell((5, 0)).unwrap().symbol(), CELL_GLYPH);
        // The last filled cell paints at x = 5 + 4*2 = 13; 14 is spacer; 15..=19 is blank.
        assert_eq!(buf.cell((13, 0)).unwrap().symbol(), CELL_GLYPH);
        assert_eq!(buf.cell((15, 0)).unwrap().symbol(), " ");
    }

    #[test]
    fn align_defaults_to_left() {
        let cells = vec![vec![5u32; 5]];
        let p = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Heatmap(HeatmapData {
                cells,
                thresholds: None,
                row_labels: None,
                col_labels: None,
            }),
        };
        let registry = Registry::with_builtins();
        let spec = RenderSpec::Short("grid_heatmap".into());
        let buf = render_to_buffer_with_spec(&p, Some(&spec), &registry, 20, 1);
        // Default align (none) = left: first glyph at column 0.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), CELL_GLYPH);
    }

    #[test]
    fn provided_thresholds_are_sorted() {
        let data = HeatmapData {
            cells: vec![vec![0, 5, 10]],
            thresholds: Some(vec![8, 2, 6, 4]),
            row_labels: None,
            col_labels: None,
        };
        let t = resolve_thresholds(&data);
        assert_eq!(t, vec![2, 4, 6, 8]);
    }
}
