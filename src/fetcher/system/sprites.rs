//! Pixel-art sprites for the `system_monitor_*` family. Battery and CPU each share an
//! outline; the bucket meaning is conveyed through the colour and length of the inner fill,
//! not through faces (an earlier draft used emoticon faces; it didn't read as ambient).
//!
//! Following the `weather/sprites.rs` pattern: ASCII grids + palette, decoded once via
//! `OnceLock`.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::payload::{PixelArtData, PixelColor};

pub const SPRITE_SIZE: usize = 16;

// ── Public API ─────────────────────────────────────────────────────

pub fn battery_sprite(ratio: f64, label: &str) -> PixelArtData {
    let bucket = bucket_index(ratio);
    let mut sprite = battery_catalog()[bucket].clone();
    sprite.label = Some(label.into());
    sprite
}

pub fn cpu_sprite(ratio: f64, label: &str) -> PixelArtData {
    let bucket = bucket_index(ratio);
    let mut sprite = cpu_catalog()[bucket].clone();
    sprite.label = Some(label.into());
    sprite
}

/// Map a ratio in `[0, 1]` to one of five buckets used by every body sprite. `0..0.2` →
/// critical, `0.2..0.4` → low, `0.4..0.6` → mid, `0.6..0.8` → good, `0.8..=1.0` → full.
/// Values outside the range clamp at the nearest end.
fn bucket_index(ratio: f64) -> usize {
    let r = ratio.clamp(0.0, 1.0);
    match r {
        r if r < 0.2 => 0,
        r if r < 0.4 => 1,
        r if r < 0.6 => 2,
        r if r < 0.8 => 3,
        _ => 4,
    }
}

// ── Catalogs ───────────────────────────────────────────────────────

fn battery_catalog() -> &'static [PixelArtData; 5] {
    static CATALOG: OnceLock<[PixelArtData; 5]> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let pal = palette();
        [
            decode(BATTERY_CRITICAL, &pal),
            decode(BATTERY_LOW, &pal),
            decode(BATTERY_MID, &pal),
            decode(BATTERY_GOOD, &pal),
            decode(BATTERY_FULL, &pal),
        ]
    })
}

fn cpu_catalog() -> &'static [PixelArtData; 5] {
    static CATALOG: OnceLock<[PixelArtData; 5]> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let pal = palette();
        [
            decode(CPU_SLEEPY, &pal),
            decode(CPU_RELAXED, &pal),
            decode(CPU_NORMAL, &pal),
            decode(CPU_BUSY, &pal),
            decode(CPU_OVERHEATING, &pal),
        ]
    })
}

fn decode(grid: &[&str], palette: &HashMap<char, PixelColor>) -> PixelArtData {
    let pixels = grid
        .iter()
        .map(|row| {
            row.chars()
                .map(|c| palette.get(&c).copied().unwrap_or(PixelColor::TRANSPARENT))
                .collect()
        })
        .collect();
    PixelArtData {
        pixels,
        label: None,
    }
}

// ── Shared palette ─────────────────────────────────────────────────

const SHELL: PixelColor = PixelColor::opaque(207, 216, 220);
const SHELL_EDGE: PixelColor = PixelColor::opaque(96, 125, 139);
const CHIP: PixelColor = PixelColor::opaque(38, 50, 56);
const CHIP_EDGE: PixelColor = PixelColor::opaque(15, 20, 25);
const PIN: PixelColor = PixelColor::opaque(189, 195, 199);
const RED: PixelColor = PixelColor::opaque(244, 67, 54);
const ORANGE: PixelColor = PixelColor::opaque(255, 152, 0);
const YELLOW: PixelColor = PixelColor::opaque(255, 235, 59);
const LIME: PixelColor = PixelColor::opaque(139, 195, 74);
const GREEN: PixelColor = PixelColor::opaque(76, 175, 80);
const BLUE: PixelColor = PixelColor::opaque(52, 152, 219);
const FLAME: PixelColor = PixelColor::opaque(255, 87, 34);

fn palette() -> HashMap<char, PixelColor> {
    HashMap::from([
        ('.', PixelColor::TRANSPARENT),
        ('S', SHELL),
        ('E', SHELL_EDGE),
        ('C', CHIP),
        ('D', CHIP_EDGE),
        ('P', PIN),
        ('R', RED),
        ('O', ORANGE),
        ('Y', YELLOW),
        ('L', LIME),
        ('G', GREEN),
        ('B', BLUE),
        ('f', FLAME),
    ])
}

// ── Battery sprites ───────────────────────────────────────────────
// Horizontal cell with a + terminal on the right. The inner bar grows left → right with the
// bucket; colour shifts critical (R) → low (O) → mid (Y) → good (L) → full (G).

const BATTERY_CRITICAL: &[&str] = &[
    "................",
    "................",
    "................",
    "................",
    "................",
    ".EEEEEEEEEEEEE..",
    ".ESRR........SE.",
    ".ESRR........SEE",
    ".ESRR........SEE",
    ".ESRR........SE.",
    ".ESSSSSSSSSSSSE.",
    ".EEEEEEEEEEEEE..",
    "................",
    "................",
    "................",
    "................",
];

const BATTERY_LOW: &[&str] = &[
    "................",
    "................",
    "................",
    "................",
    "................",
    ".EEEEEEEEEEEEE..",
    ".ESOOOO......SE.",
    ".ESOOOO......SEE",
    ".ESOOOO......SEE",
    ".ESOOOO......SE.",
    ".ESSSSSSSSSSSSE.",
    ".EEEEEEEEEEEEE..",
    "................",
    "................",
    "................",
    "................",
];

const BATTERY_MID: &[&str] = &[
    "................",
    "................",
    "................",
    "................",
    "................",
    ".EEEEEEEEEEEEE..",
    ".ESYYYYYYY...SE.",
    ".ESYYYYYYY...SEE",
    ".ESYYYYYYY...SEE",
    ".ESYYYYYYY...SE.",
    ".ESSSSSSSSSSSSE.",
    ".EEEEEEEEEEEEE..",
    "................",
    "................",
    "................",
    "................",
];

const BATTERY_GOOD: &[&str] = &[
    "................",
    "................",
    "................",
    "................",
    "................",
    ".EEEEEEEEEEEEE..",
    ".ESLLLLLLLLL.SE.",
    ".ESLLLLLLLLL.SEE",
    ".ESLLLLLLLLL.SEE",
    ".ESLLLLLLLLL.SE.",
    ".ESSSSSSSSSSSSE.",
    ".EEEEEEEEEEEEE..",
    "................",
    "................",
    "................",
    "................",
];

const BATTERY_FULL: &[&str] = &[
    "................",
    "................",
    "................",
    "................",
    "................",
    ".EEEEEEEEEEEEE..",
    ".ESGGGGGGGGGGGE.",
    ".ESGGGGGGGGGGGEE",
    ".ESGGGGGGGGGGGEE",
    ".ESGGGGGGGGGGGE.",
    ".ESSSSSSSSSSSSE.",
    ".EEEEEEEEEEEEE..",
    "................",
    "................",
    "................",
    "................",
];

// ── CPU sprites ───────────────────────────────────────────────────
// Chip silhouette with top/bottom pin rows and small left/right pins. The 6x4 core in the
// middle is what changes colour with load: cool blue when sleepy through warm red when
// overheating. Overheating adds small flame tufts above the chip package.

const CPU_SLEEPY: &[&str] = &[
    "................",
    "................",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "..DDDDDDDDDDDD..",
    "..DCCCCCCCCCCD..",
    "..DCCBBBBBBCCD..",
    "..DCCBBBBBBCCD..",
    "..DCCBBBBBBCCD..",
    "..DCCBBBBBBCCD..",
    "..DCCCCCCCCCCD..",
    "..DDDDDDDDDDDD..",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "................",
    "................",
];

const CPU_RELAXED: &[&str] = &[
    "................",
    "................",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "..DDDDDDDDDDDD..",
    "..DCCCCCCCCCCD..",
    "..DCCLLLLLLCCD..",
    "..DCCLLLLLLCCD..",
    "..DCCLLLLLLCCD..",
    "..DCCLLLLLLCCD..",
    "..DCCCCCCCCCCD..",
    "..DDDDDDDDDDDD..",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "................",
    "................",
];

const CPU_NORMAL: &[&str] = &[
    "................",
    "................",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "..DDDDDDDDDDDD..",
    "..DCCCCCCCCCCD..",
    "..DCCYYYYYYCCD..",
    "..DCCYYYYYYCCD..",
    "..DCCYYYYYYCCD..",
    "..DCCYYYYYYCCD..",
    "..DCCCCCCCCCCD..",
    "..DDDDDDDDDDDD..",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "................",
    "................",
];

const CPU_BUSY: &[&str] = &[
    "................",
    "................",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "..DDDDDDDDDDDD..",
    "..DCCCCCCCCCCD..",
    "..DCCOOOOOOCCD..",
    "..DCCOOOOOOCCD..",
    "..DCCOOOOOOCCD..",
    "..DCCOOOOOOCCD..",
    "..DCCCCCCCCCCD..",
    "..DDDDDDDDDDDD..",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "................",
    "................",
];

const CPU_OVERHEATING: &[&str] = &[
    "................",
    "................",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "..DDDDDDDDDDDD..",
    "..DCCCCCCCCCCD..",
    "..DCCRRRRRRCCD..",
    "..DCCRRRRRRCCD..",
    "..DCCRRRRRRCCD..",
    "..DCCRRRRRRCCD..",
    "..DCCCCCCCCCCD..",
    "..DDDDDDDDDDDD..",
    "...P..P..P..P...",
    "...P..P..P..P...",
    "................",
    "................",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_16x16(grid: &[&str]) {
        assert_eq!(grid.len(), SPRITE_SIZE, "row count");
        for (i, row) in grid.iter().enumerate() {
            assert_eq!(row.chars().count(), SPRITE_SIZE, "row {i} width");
        }
    }

    #[test]
    fn battery_grids_are_16x16() {
        for g in [
            BATTERY_CRITICAL,
            BATTERY_LOW,
            BATTERY_MID,
            BATTERY_GOOD,
            BATTERY_FULL,
        ] {
            assert_16x16(g);
        }
    }

    #[test]
    fn cpu_grids_are_16x16() {
        for g in [
            CPU_SLEEPY,
            CPU_RELAXED,
            CPU_NORMAL,
            CPU_BUSY,
            CPU_OVERHEATING,
        ] {
            assert_16x16(g);
        }
    }

    #[test]
    fn bucket_thresholds_match_documentation() {
        for (ratio, expected) in [
            (0.0, 0),
            (0.19, 0),
            (0.2, 1),
            (0.39, 1),
            (0.4, 2),
            (0.59, 2),
            (0.6, 3),
            (0.79, 3),
            (0.8, 4),
            (1.0, 4),
            (-0.5, 0),
            (5.0, 4),
        ] {
            assert_eq!(bucket_index(ratio), expected, "ratio = {ratio}");
        }
    }

    #[test]
    fn battery_sprite_label_round_trips() {
        let s = battery_sprite(0.5, "55%");
        assert_eq!(s.label.as_deref(), Some("55%"));
        assert_eq!(s.pixels.len(), SPRITE_SIZE);
    }

    #[test]
    fn cpu_sprite_label_round_trips() {
        let s = cpu_sprite(0.95, "overheating");
        assert_eq!(s.label.as_deref(), Some("overheating"));
        assert_eq!(s.pixels.len(), SPRITE_SIZE);
    }

    #[test]
    fn every_palette_character_is_declared() {
        let pal = palette();
        for grid in [
            BATTERY_CRITICAL,
            BATTERY_LOW,
            BATTERY_MID,
            BATTERY_GOOD,
            BATTERY_FULL,
            CPU_SLEEPY,
            CPU_RELAXED,
            CPU_NORMAL,
            CPU_BUSY,
            CPU_OVERHEATING,
        ] {
            for (y, row) in grid.iter().enumerate() {
                for (x, ch) in row.chars().enumerate() {
                    assert!(
                        pal.contains_key(&ch),
                        "char {ch:?} at ({x},{y}) missing palette entry"
                    );
                }
            }
        }
    }
}
