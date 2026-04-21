use ratatui::{Frame, buffer::Buffer, layout::Rect, style::Color};

use crate::payload::{Body, HeatmapData};

use super::{RenderOptions, Renderer, Shape};

/// 2D intensity grid — GitHub-style contribution graph and any other daily/periodic heatmap.
/// Each cell takes two terminal columns and one row so the grid reads as "dots of equal width"
/// despite the 1:2 character aspect ratio. Buckets each cell's value into one of 5 intensity
/// levels; level 0 is dim background, level 4 is full-saturation theme color. Thresholds come
/// from the payload when provided, otherwise fall back to an auto-quartile split of the data.
pub struct HeatmapRenderer;

const LEVELS: usize = 5;
const CELL_W: u16 = 2;
const CELL_H: u16 = 1;

/// Background gradient for the contribution-graph "grass" palette, level 0 → 4. Picked to be
/// readable on both dark and light terminals without a theme system in place yet; will be
/// replaced by theme lookups once #17 lands.
const PALETTE: [Color; LEVELS] = [
    Color::Rgb(0x15, 0x1B, 0x23),
    Color::Rgb(0x0E, 0x44, 0x29),
    Color::Rgb(0x00, 0x6D, 0x32),
    Color::Rgb(0x26, 0xA6, 0x41),
    Color::Rgb(0x39, 0xD3, 0x53),
];

impl Renderer for HeatmapRenderer {
    fn name(&self) -> &str {
        "heatmap"
    }
    fn accepts(&self) -> &[Shape] {
        &[Shape::Heatmap]
    }
    fn render(&self, frame: &mut Frame, area: Rect, body: &Body, _opts: &RenderOptions) {
        if let Body::Heatmap(d) = body {
            render_heatmap(frame, area, d);
        }
    }
}

fn render_heatmap(frame: &mut Frame, area: Rect, data: &HeatmapData) {
    if data.cells.is_empty() || data.cells[0].is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let thresholds = resolve_thresholds(data);
    paint_cells(frame.buffer_mut(), area, data, &thresholds);
}

fn paint_cells(buf: &mut Buffer, area: Rect, data: &HeatmapData, thresholds: &[u32]) {
    let rows = data.cells.len();
    let cols = data.cells[0].len();
    let max_visible_rows = (area.height / CELL_H) as usize;
    let max_visible_cols = (area.width / CELL_W) as usize;
    let visible_rows = rows.min(max_visible_rows);
    let visible_cols = cols.min(max_visible_cols);
    // Right-align columns so "the most recent N weeks" is what survives when the slot is
    // narrower than the full year. Row alignment is top-anchored (weekday order is fixed).
    let col_offset = cols.saturating_sub(visible_cols);
    for r in 0..visible_rows {
        for c in 0..visible_cols {
            let value = data.cells[r][c + col_offset];
            let level = bucket(value, thresholds);
            let color = PALETTE[level];
            let x = area.x + (c as u16) * CELL_W;
            let y = area.y + (r as u16) * CELL_H;
            for dx in 0..CELL_W {
                for dy in 0..CELL_H {
                    if let Some(cell) = buf.cell_mut((x + dx, y + dy)) {
                        cell.set_bg(color);
                        cell.set_symbol(" ");
                    }
                }
            }
        }
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
        let spec = RenderSpec::Short("heatmap".into());
        render_to_buffer_with_spec(&payload(cells), Some(&spec), &registry, w, h)
    }

    #[test]
    fn empty_cells_no_panic() {
        let _ = render(vec![], 20, 7);
        let _ = render(vec![vec![]], 20, 7);
    }

    #[test]
    fn renders_grid_without_panicking() {
        let cells: Vec<Vec<u32>> = (0..7).map(|r| (0..20).map(|c| (r * c) as u32).collect()).collect();
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
        // The rightmost cell must have a non-default bg.
        let right = buf.cell((38, 0)).unwrap();
        assert_ne!(
            right.bg,
            ratatui::style::Color::Reset,
            "rightmost cell should be painted"
        );
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
