//! Built-in theme presets. Each function returns a fully-populated [`Theme`]; `[theme]
//! preset = "name"` selects one as the base, and any sibling keys in `[theme]` override
//! individual tokens on top (see [`super::Theme::from_config`]).
//!
//! Adding a preset: pick the palette's hex values, map them to semantic tokens by the
//! conventions below, add the `fn`, list it in [`by_name`], and add the name to the
//! `KNOWN` slice. No registry — just functions — keeps each preset self-contained and
//! grep-able.
//!
//! Mapping conventions (applied uniformly across presets so swapping one for another gives a
//! predictable shift rather than a surprise):
//! - `bg` / `bg_subtle` = palette background + one step lifted.
//! - `text` = palette foreground; `text_dim` = comment tone; `text_secondary` = a step
//!   brighter than `text_dim`.
//! - `panel_border` = the palette's "panel / surface" tone (slightly dimmer than `text`).
//! - `panel_title` = the palette's primary accent blue/purple.
//! - `status_ok` / `status_warn` / `status_error` = the palette's green / yellow / red.
//! - `accent_today` = yellow; `accent_event` = cyan.
//! - `palette_series` = 10-entry cycle drawn from the palette's accent colours.
//! - `palette_heatmap` = 5-step gradient from a near-background tone up to the palette's
//!   green.

use ratatui::style::Color;

use super::Theme;

/// Names of every built-in preset, in the order they appear below. Used for
/// `[theme] preset = "..."` lookups and the "unknown preset" error message.
/// `"splash"` is the signature default, reachable either by omitting `[theme]
/// preset = "..."` entirely or by spelling it out. `"default"` is a generic
/// alias for the same thing — kept so configs using preset-switching harnesses
/// don't have to special-case the built-in.
pub const KNOWN: &[&str] = &[
    "splash",
    "default",
    "tokyo_night",
    "nord",
    "dracula",
    "gruvbox_dark",
    "catppuccin_mocha",
];

/// Resolve a preset by name. Returns `None` for unknown names — the caller ([`super::Theme::
/// from_config`]) surfaces that as a config error rather than silently falling through to the
/// default theme.
pub fn by_name(name: &str) -> Option<Theme> {
    match name {
        "splash" | "default" => Some(splash()),
        "tokyo_night" => Some(tokyo_night()),
        "nord" => Some(nord()),
        "dracula" => Some(dracula()),
        "gruvbox_dark" => Some(gruvbox_dark()),
        "catppuccin_mocha" => Some(catppuccin_mocha()),
        _ => None,
    }
}

/// The "Splash" signature palette — coral + cyan-teal on deep-ocean navy. Identical to
/// [`Theme::default`], but exposed as a named function for symmetry with the other
/// presets and so it appears in the preset catalog.
pub fn splash() -> Theme {
    Theme::default()
}

pub fn tokyo_night() -> Theme {
    Theme {
        bg: hex(0x1a, 0x1b, 0x26),
        bg_subtle: hex(0x24, 0x28, 0x3b),
        text: hex(0xc0, 0xca, 0xf5),
        panel_border: hex(0x41, 0x48, 0x68),
        panel_title: hex(0x7a, 0xa2, 0xf7),
        status_ok: hex(0x9e, 0xce, 0x6a),
        status_warn: hex(0xe0, 0xaf, 0x68),
        status_error: hex(0xf7, 0x76, 0x8e),
        text_dim: hex(0x56, 0x5f, 0x89),
        text_secondary: hex(0x9a, 0xa5, 0xce),
        accent_today: hex(0xe0, 0xaf, 0x68),
        accent_event: hex(0x7d, 0xcf, 0xff),
        palette_series: vec![
            hex(0xf7, 0x76, 0x8e),
            hex(0x7a, 0xa2, 0xf7),
            hex(0x9e, 0xce, 0x6a),
            hex(0xe0, 0xaf, 0x68),
            hex(0xbb, 0x9a, 0xf7),
            hex(0x7d, 0xcf, 0xff),
            hex(0xff, 0x9e, 0x64),
            hex(0x2a, 0xc3, 0xde),
            hex(0x73, 0xda, 0xca),
            hex(0xc0, 0xca, 0xf5),
        ],
        palette_heatmap: vec![
            hex(0x1f, 0x23, 0x35),
            hex(0x3d, 0x5a, 0x3d),
            hex(0x55, 0x7a, 0x4a),
            hex(0x7a, 0xa6, 0x54),
            hex(0x9e, 0xce, 0x6a),
        ],
    }
}

pub fn nord() -> Theme {
    Theme {
        bg: hex(0x2e, 0x34, 0x40),
        bg_subtle: hex(0x3b, 0x42, 0x52),
        text: hex(0xd8, 0xde, 0xe9),
        panel_border: hex(0x4c, 0x56, 0x6a),
        panel_title: hex(0x88, 0xc0, 0xd0),
        status_ok: hex(0xa3, 0xbe, 0x8c),
        status_warn: hex(0xeb, 0xcb, 0x8b),
        status_error: hex(0xbf, 0x61, 0x6a),
        text_dim: hex(0x43, 0x4c, 0x5e),
        text_secondary: hex(0xe5, 0xe9, 0xf0),
        accent_today: hex(0xeb, 0xcb, 0x8b),
        accent_event: hex(0x8f, 0xbc, 0xbb),
        palette_series: vec![
            hex(0x88, 0xc0, 0xd0),
            hex(0xa3, 0xbe, 0x8c),
            hex(0xeb, 0xcb, 0x8b),
            hex(0xbf, 0x61, 0x6a),
            hex(0xb4, 0x8e, 0xad),
            hex(0x8f, 0xbc, 0xbb),
            hex(0xd0, 0x87, 0x70),
            hex(0x81, 0xa1, 0xc1),
            hex(0x5e, 0x81, 0xac),
            hex(0xe5, 0xe9, 0xf0),
        ],
        palette_heatmap: vec![
            hex(0x3b, 0x42, 0x52),
            hex(0x4c, 0x56, 0x6a),
            hex(0x5e, 0x81, 0xac),
            hex(0x81, 0xa1, 0xc1),
            hex(0xa3, 0xbe, 0x8c),
        ],
    }
}

pub fn dracula() -> Theme {
    Theme {
        bg: hex(0x28, 0x2a, 0x36),
        bg_subtle: hex(0x44, 0x47, 0x5a),
        text: hex(0xf8, 0xf8, 0xf2),
        panel_border: hex(0x44, 0x47, 0x5a),
        panel_title: hex(0xbd, 0x93, 0xf9),
        status_ok: hex(0x50, 0xfa, 0x7b),
        status_warn: hex(0xf1, 0xfa, 0x8c),
        status_error: hex(0xff, 0x55, 0x55),
        text_dim: hex(0x62, 0x72, 0xa4),
        text_secondary: hex(0xf8, 0xf8, 0xf2),
        accent_today: hex(0xf1, 0xfa, 0x8c),
        accent_event: hex(0x8b, 0xe9, 0xfd),
        palette_series: vec![
            hex(0xff, 0x79, 0xc6),
            hex(0x8b, 0xe9, 0xfd),
            hex(0x50, 0xfa, 0x7b),
            hex(0xff, 0xb8, 0x6c),
            hex(0xbd, 0x93, 0xf9),
            hex(0xf1, 0xfa, 0x8c),
            hex(0xff, 0x55, 0x55),
            hex(0x62, 0x72, 0xa4),
            hex(0x44, 0x47, 0x5a),
            hex(0xf8, 0xf8, 0xf2),
        ],
        palette_heatmap: vec![
            hex(0x28, 0x2a, 0x36),
            hex(0x37, 0x4c, 0x3e),
            hex(0x3d, 0x7a, 0x52),
            hex(0x3f, 0xb5, 0x65),
            hex(0x50, 0xfa, 0x7b),
        ],
    }
}

pub fn gruvbox_dark() -> Theme {
    Theme {
        bg: hex(0x28, 0x28, 0x28),
        bg_subtle: hex(0x3c, 0x38, 0x36),
        text: hex(0xeb, 0xdb, 0xb2),
        panel_border: hex(0x7c, 0x6f, 0x64),
        panel_title: hex(0xfa, 0xbd, 0x2f),
        status_ok: hex(0xb8, 0xbb, 0x26),
        status_warn: hex(0xfa, 0xbd, 0x2f),
        status_error: hex(0xfb, 0x49, 0x34),
        text_dim: hex(0x66, 0x5c, 0x54),
        text_secondary: hex(0xa8, 0x99, 0x84),
        accent_today: hex(0xfa, 0xbd, 0x2f),
        accent_event: hex(0x83, 0xa5, 0x98),
        palette_series: vec![
            hex(0xfb, 0x49, 0x34),
            hex(0x83, 0xa5, 0x98),
            hex(0xb8, 0xbb, 0x26),
            hex(0xfa, 0xbd, 0x2f),
            hex(0xd3, 0x86, 0x9b),
            hex(0x8e, 0xc0, 0x7c),
            hex(0xfe, 0x80, 0x19),
            hex(0x45, 0x85, 0x88),
            hex(0xb1, 0x62, 0x86),
            hex(0xeb, 0xdb, 0xb2),
        ],
        palette_heatmap: vec![
            hex(0x28, 0x28, 0x28),
            hex(0x3c, 0x38, 0x36),
            hex(0x68, 0x9d, 0x6a),
            hex(0x98, 0x97, 0x1a),
            hex(0xb8, 0xbb, 0x26),
        ],
    }
}

pub fn catppuccin_mocha() -> Theme {
    Theme {
        bg: hex(0x1e, 0x1e, 0x2e),
        bg_subtle: hex(0x31, 0x32, 0x44),
        text: hex(0xcd, 0xd6, 0xf4),
        panel_border: hex(0x45, 0x47, 0x5a),
        panel_title: hex(0x89, 0xb4, 0xfa),
        status_ok: hex(0xa6, 0xe3, 0xa1),
        status_warn: hex(0xf9, 0xe2, 0xaf),
        status_error: hex(0xf3, 0x8b, 0xa8),
        text_dim: hex(0x58, 0x5b, 0x70),
        text_secondary: hex(0xa6, 0xad, 0xc8),
        accent_today: hex(0xf9, 0xe2, 0xaf),
        accent_event: hex(0x94, 0xe2, 0xd5),
        palette_series: vec![
            hex(0xf3, 0x8b, 0xa8),
            hex(0x89, 0xb4, 0xfa),
            hex(0xa6, 0xe3, 0xa1),
            hex(0xf9, 0xe2, 0xaf),
            hex(0xcb, 0xa6, 0xf7),
            hex(0x94, 0xe2, 0xd5),
            hex(0xfa, 0xb3, 0x87),
            hex(0x74, 0xc7, 0xec),
            hex(0xf5, 0xc2, 0xe7),
            hex(0xcd, 0xd6, 0xf4),
        ],
        palette_heatmap: vec![
            hex(0x1e, 0x1e, 0x2e),
            hex(0x40, 0x6a, 0x3d),
            hex(0x60, 0x92, 0x4e),
            hex(0x80, 0xba, 0x60),
            hex(0xa6, 0xe3, 0xa1),
        ],
    }
}

const fn hex(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_name_resolves() {
        // `KNOWN` is the source of truth consulted by error messages; if `by_name` can't
        // resolve a listed name, config validation would point users at a preset they can't
        // actually select.
        for name in KNOWN {
            assert!(
                by_name(name).is_some(),
                "preset {name} listed but not resolvable"
            );
        }
    }

    #[test]
    fn unknown_name_returns_none() {
        assert!(by_name("doesnotexist").is_none());
    }

    #[test]
    fn splash_and_default_both_resolve_to_the_same_theme() {
        // Alias check: users should get identical palettes whether they write the
        // signature name or the generic keyword, so preset-switching UIs don't have
        // to branch on which spelling is current.
        let splash = by_name("splash").unwrap();
        let default = by_name("default").unwrap();
        let implicit = Theme::default();
        assert_eq!(splash.bg, default.bg);
        assert_eq!(splash.bg, implicit.bg);
        assert_eq!(splash.panel_title, implicit.panel_title);
    }

    #[test]
    fn every_preset_populates_all_palette_arrays() {
        // Guard against a future preset accidentally shipping an empty series / ramp, which
        // would silently fall back to `Color::Cyan` / the default ramp at render time.
        for name in KNOWN {
            let t = by_name(name).unwrap();
            assert!(!t.palette_series.is_empty(), "{name}: series is empty");
            assert!(
                !t.palette_heatmap.is_empty(),
                "{name}: heatmap_ramp is empty"
            );
        }
    }
}
