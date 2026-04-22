//! Semantic colour keys. Renderers look up named keys rather than hard-coding `Color::Green`,
//! so users can repaint the whole splash by overriding a handful of tokens in
//! `[theme]`.
//!
//! Shape of the contract:
//! - [`ColorKey`] is a static metadata record a renderer declares via
//!   [`Renderer::color_keys`](crate::render::Renderer::color_keys). The xtask catalog picks those
//!   up so the docs page for every renderer lists the tokens it uses.
//! - [`Theme`] is the resolved palette passed to every `render()` call. Callers build it once
//!   from [`ThemeConfig`] (TOML) via [`Theme::from_config`].
//!
//! ## Default theme — "Splash"
//!
//! Motif: **sunrise over deep ocean**. Every token is tuned so the palette reads like one
//! place rather than a grab-bag of accents. Two heroes anchor it — a warm coral for the
//! rising sun (`panel_title`, errors, hottest heatmap cell) and a cool cyan-teal for
//! sunlit shallows (`accent_event`, calendar markers). `text_dim` and `text_secondary`
//! lean teal so chrome feels like looking up through water. The `palette_series` cycle
//! stays on the coral↔ocean axis; `palette_heatmap` climbs night → shallow → sunrise →
//! dawn.
//!
//! Token families follow a strict prefix scheme so TOML stays grep-able and new tokens
//! slot into an obvious home:
//!
//! - `bg` / `bg_subtle` — painted surfaces.
//! - `text` / `text_dim` / `text_secondary` — body copy and its dimmer variants.
//! - `panel_border` / `panel_title` — panel chrome.
//! - `status_ok` / `status_warn` / `status_error` — badge triad.
//! - `accent_today` / `accent_event` — calendar / scatter markers.
//! - `palette_series` / `palette_heatmap` — multi-colour palettes.
//!
//! Every token is opinionated — the splash owns the entire surface rather than leaking
//! terminal colours through unpainted cells. Users on light terminals (or anyone who'd
//! rather keep their terminal palette) can revert any token with `[theme] <name> =
//! "reset"` — `bg = "reset"`, `text = "reset"`, etc. — which falls back to `Color::Reset`
//! for that slot only.

use ratatui::style::Color;
use serde::Deserialize;

pub mod presets;

/// Metadata describing one semantic colour token a renderer consumes. Lives next to the
/// renderer's declaration so it can't drift out of sync with what the code actually reads.
/// The xtask docs generator renders these into each renderer's reference page.
#[derive(Debug, Clone, Copy)]
pub struct ColorKey {
    /// TOML key under `[theme]`. Snake_case.
    pub name: &'static str,
    /// One-line human description — rendered verbatim into docs.
    pub description: &'static str,
}

// ── Single-colour keys ─────────────────────────────────────────────────────────────────────
pub const BG: ColorKey = ColorKey {
    name: "bg",
    description: "Viewport background. Painted across the entire splash area before widgets draw. The default is a deep-ocean navy; set `bg = \"reset\"` to inherit the terminal's own background instead (recommended on light terminals).",
};
pub const BG_SUBTLE: ColorKey = ColorKey {
    name: "bg_subtle",
    description: "Secondary surface painted when a `[[row]]` or `[[row.child]]` opts in with `bg = \"subtle\"`. One step off `bg` — for headers, callouts, or any slot that should visually separate from the main content band.",
};
pub const TEXT: ColorKey = ColorKey {
    name: "text",
    description: "Primary body text colour for plain renderers (text_plain, text_ascii, list_plain, animated_typewriter). The top step of the chrome ladder — brighter than `text_secondary`, reads as diffuse daylight on the ocean surface. Set `text = \"reset\"` to inherit the terminal's own foreground instead.",
};
pub const PANEL_BORDER: ColorKey = ColorKey {
    name: "panel_border",
    description: "Panel border colour. Defaults to a muted ocean teal-gray that sits between `bg` and `text_secondary` — visible as structure without competing with the coral `panel_title`. Set `panel_border = \"reset\"` to inherit the terminal's own foreground instead.",
};
pub const PANEL_TITLE: ColorKey = ColorKey {
    name: "panel_title",
    description: "Panel title colour. The coral hero of the Splash palette; set `panel_title = \"reset\"` to inherit the terminal's own foreground.",
};
pub const STATUS_OK: ColorKey = ColorKey {
    name: "status_ok",
    description: "Healthy / passing status (badges, table rows, timeline titles).",
};
pub const STATUS_WARN: ColorKey = ColorKey {
    name: "status_warn",
    description: "Degraded / warning status — visually distinct from both ok and error.",
};
pub const STATUS_ERROR: ColorKey = ColorKey {
    name: "status_error",
    description: "Failing / error status — reserved for attention-grabbing failures.",
};
pub const TEXT_DIM: ColorKey = ColorKey {
    name: "text_dim",
    description: "Barely-visible chrome text: timeline date prefixes, empty-state placeholder, clip hint. Replaces the old `Modifier::DIM` usage — set an explicit colour so rendering stays consistent across terminals (some render DIM as invisible, some as a no-op).",
};
pub const TEXT_SECONDARY: ColorKey = ColorKey {
    name: "text_secondary",
    description: "Slightly-less-dim secondary text, e.g. the detail line under a timeline entry.",
};
pub const ACCENT_TODAY: ColorKey = ColorKey {
    name: "accent_today",
    description: "Calendar \"today\" marker.",
};
pub const ACCENT_EVENT: ColorKey = ColorKey {
    name: "accent_event",
    description: "Calendar event marker and default scatter-plot dot colour.",
};

// ── Multi-colour palettes (`palette_*` family) ─────────────────────────────────────────────
pub const PALETTE_SERIES: ColorKey = ColorKey {
    name: "palette_series",
    description: "Palette cycled per-slice / per-series by chart_pie / chart_line / chart_scatter. TOML takes an array of colours.",
};
pub const PALETTE_HEATMAP: ColorKey = ColorKey {
    name: "palette_heatmap",
    description: "5-step intensity gradient for grid_heatmap (level 0 → level 4). TOML array.",
};

/// Resolved palette handed to each `render()` call. Immutable within a draw cycle.
#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub bg_subtle: Color,
    pub text: Color,
    pub panel_border: Color,
    pub panel_title: Color,
    pub status_ok: Color,
    pub status_warn: Color,
    pub status_error: Color,
    pub text_dim: Color,
    pub text_secondary: Color,
    pub accent_today: Color,
    pub accent_event: Color,
    pub palette_series: Vec<Color>,
    pub palette_heatmap: Vec<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        // Splash: sunrise over deep ocean. Hero warm (coral) + hero cool (cyan-teal)
        // are the anchors; chrome leans teal so it feels like submerged light; accents
        // all live on the coral↔ocean axis so adjacent slices / stacked widgets read as
        // one place instead of a palette swatch.
        Self {
            bg: Color::Rgb(0x0e, 0x17, 0x2a), // deep-ocean navy — twilight
            bg_subtle: Color::Rgb(0x17, 0x24, 0x3b), // lift: horizon about to break
            text: Color::Rgb(0xc5, 0xd2, 0xdc), // diffuse daylight on water
            panel_border: Color::Rgb(0x4a, 0x6b, 0x75), // ocean teal-gray — panel grid
            panel_title: Color::Rgb(0xff, 0x8c, 0x7a), // coral — the rising sun
            status_ok: Color::Rgb(0x7d, 0xe0, 0xb5), // mint — first blue-green light
            status_warn: Color::Rgb(0xff, 0xc6, 0x6b), // amber — sunrise warning
            status_error: Color::Rgb(0xff, 0x5e, 0x7a), // coral-red — blood moon
            text_dim: Color::Rgb(0x2d, 0x47, 0x57), // teal-dark — deep water
            text_secondary: Color::Rgb(0x7e, 0xa0, 0xb5), // teal-steel — sunlit haze
            accent_today: Color::Rgb(0xff, 0xc6, 0x6b), // amber — today is dawn
            accent_event: Color::Rgb(0x5e, 0xd8, 0xe0), // cyan-teal — the sea
            palette_series: default_series(),
            palette_heatmap: default_heatmap_ramp(),
        }
    }
}

impl Theme {
    /// nth series colour with wrap-around; empty palette falls back to the default cyan.
    pub fn series_color(&self, i: usize) -> Color {
        if self.palette_series.is_empty() {
            return Color::Cyan;
        }
        self.palette_series[i % self.palette_series.len()]
    }

    /// Heatmap level in `0..5`, saturating at both ends. Empty ramp falls back to default.
    pub fn heatmap_level(&self, level: usize) -> Color {
        let ramp = if self.palette_heatmap.is_empty() {
            return default_heatmap_ramp()[level.min(4)];
        } else {
            &self.palette_heatmap
        };
        ramp[level.min(ramp.len() - 1)]
    }

    /// Resolve a theme from config. `cfg.preset` picks the base palette (falling back to
    /// [`Theme::default`] when absent or unknown); individual fields then overlay on top so
    /// users can write `preset = "nord"` and tweak one or two tokens without copying the
    /// whole palette. An unknown preset name is logged on stderr and treated as no preset —
    /// rendering still proceeds with the default rather than crashing the splash.
    pub fn from_config(cfg: &ThemeConfig) -> Self {
        let mut t = match cfg.preset.as_deref() {
            None => Self::default(),
            Some(name) => match presets::by_name(name) {
                Some(theme) => theme,
                None => {
                    eprintln!(
                        "splashboard: unknown theme preset {name:?}; known: {:?}",
                        presets::KNOWN
                    );
                    Self::default()
                }
            },
        };
        if let Some(c) = cfg.bg {
            t.bg = c;
        }
        if let Some(c) = cfg.bg_subtle {
            t.bg_subtle = c;
        }
        if let Some(c) = cfg.text {
            t.text = c;
        }
        if let Some(c) = cfg.panel_border {
            t.panel_border = c;
        }
        if let Some(c) = cfg.panel_title {
            t.panel_title = c;
        }
        if let Some(c) = cfg.status_ok {
            t.status_ok = c;
        }
        if let Some(c) = cfg.status_warn {
            t.status_warn = c;
        }
        if let Some(c) = cfg.status_error {
            t.status_error = c;
        }
        if let Some(c) = cfg.text_dim {
            t.text_dim = c;
        }
        if let Some(c) = cfg.text_secondary {
            t.text_secondary = c;
        }
        if let Some(c) = cfg.accent_today {
            t.accent_today = c;
        }
        if let Some(c) = cfg.accent_event {
            t.accent_event = c;
        }
        if let Some(series) = &cfg.palette_series
            && !series.is_empty()
        {
            t.palette_series = series.clone();
        }
        if let Some(ramp) = &cfg.palette_heatmap
            && !ramp.is_empty()
        {
            t.palette_heatmap = ramp.clone();
        }
        t
    }
}

/// 10-entry series palette cycled by chart_pie / chart_line / chart_scatter. Every colour
/// sits on the coral↔ocean axis — no rogue lime/violet — so adjacent slices read as
/// "different colours in the same scene" rather than "random crayons". Ordered warm↔cool
/// so the first two slots give you the canonical coral/teal pair.
fn default_series() -> Vec<Color> {
    vec![
        Color::Rgb(0xff, 0x8c, 0x7a), // coral — hero warm
        Color::Rgb(0x5e, 0xd8, 0xe0), // cyan-teal — hero cool
        Color::Rgb(0xff, 0xc6, 0x6b), // amber
        Color::Rgb(0x7e, 0xc8, 0xd8), // sky-teal
        Color::Rgb(0xff, 0x5e, 0x7a), // coral-red
        Color::Rgb(0x7d, 0xe0, 0xb5), // mint
        Color::Rgb(0xf0, 0xa4, 0x88), // rose-gold
        Color::Rgb(0x3c, 0xa5, 0xa8), // deep teal
        Color::Rgb(0xff, 0xb8, 0x9e), // warm peach
        Color::Rgb(0x8a, 0x9a, 0xc4), // dusty lavender — the one cool-neutral
    ]
}

/// 5-step heatmap ramp: a slow climb from deep-ocean night through shallow-teal daylight
/// into coral sunrise and amber dawn. No purple/wine middle — the world is water and
/// fire, nothing else. "No activity" sinks into the navy bg; peak activity radiates
/// warm amber, making a contribution heatmap instantly recognizable as splashboard's.
fn default_heatmap_ramp() -> Vec<Color> {
    vec![
        Color::Rgb(0x0a, 0x14, 0x24), // deep-night — sits below the bg
        Color::Rgb(0x0d, 0x3f, 0x4a), // deep teal — bathypelagic
        Color::Rgb(0x2a, 0x8c, 0x8f), // mid-teal — shallow water
        Color::Rgb(0xff, 0x8c, 0x7a), // coral — horizon catches fire
        Color::Rgb(0xff, 0xc6, 0x6b), // amber — dawn, hottest
    ]
}

/// TOML representation of `[theme]`. Every field optional — omitted keys fall back to the
/// built-in default. `ratatui::Color` deserializes from both named colours (`"green"`,
/// `"dark_gray"`) and `"#RRGGBB"` hex, so users don't need to think about which form.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    /// Name of a built-in palette to use as the base. Sibling keys override individual
    /// tokens on top. See [`presets::KNOWN`] for the list.
    pub preset: Option<String>,
    pub bg: Option<Color>,
    pub bg_subtle: Option<Color>,
    pub text: Option<Color>,
    pub panel_border: Option<Color>,
    pub panel_title: Option<Color>,
    pub status_ok: Option<Color>,
    pub status_warn: Option<Color>,
    pub status_error: Option<Color>,
    pub text_dim: Option<Color>,
    pub text_secondary: Option<Color>,
    pub accent_today: Option<Color>,
    pub accent_event: Option<Color>,
    pub palette_series: Option<Vec<Color>>,
    pub palette_heatmap: Option<Vec<Color>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_locks_in_signature_palette() {
        // "Splash" is the out-of-the-box identity — changing a value here is a visual
        // regression for every fresh install. Locks in the exact sRGB values so
        // accidental edits are caught in CI. Any real change to the palette should
        // update this test deliberately.
        let t = Theme::default();
        assert_eq!(t.bg, Color::Rgb(0x0e, 0x17, 0x2a), "deep-ocean navy");
        assert_eq!(t.bg_subtle, Color::Rgb(0x17, 0x24, 0x3b));
        assert_eq!(
            t.panel_border,
            Color::Rgb(0x4a, 0x6b, 0x75),
            "ocean teal-gray"
        );
        assert_eq!(t.text, Color::Rgb(0xc5, 0xd2, 0xdc), "diffuse daylight");
        assert_eq!(
            t.panel_title,
            Color::Rgb(0xff, 0x8c, 0x7a),
            "coral — rising sun"
        );
        assert_eq!(
            t.accent_event,
            Color::Rgb(0x5e, 0xd8, 0xe0),
            "cyan-teal — the sea"
        );
        assert_eq!(t.text_dim, Color::Rgb(0x2d, 0x47, 0x57), "teal-dark chrome");
        assert_eq!(t.text_secondary, Color::Rgb(0x7e, 0xa0, 0xb5));
        assert_eq!(t.status_ok, Color::Rgb(0x7d, 0xe0, 0xb5));
        assert_eq!(t.status_warn, Color::Rgb(0xff, 0xc6, 0x6b));
        assert_eq!(t.status_error, Color::Rgb(0xff, 0x5e, 0x7a));
        // Series anchors to the coral/teal pair in slots 0/1.
        assert_eq!(t.palette_series.len(), 10);
        assert_eq!(t.palette_series[0], Color::Rgb(0xff, 0x8c, 0x7a));
        assert_eq!(t.palette_series[1], Color::Rgb(0x5e, 0xd8, 0xe0));
        // Ramp: night → shallow → sunrise, no purple middle.
        assert_eq!(t.palette_heatmap.len(), 5);
        assert_eq!(t.palette_heatmap[0], Color::Rgb(0x0a, 0x14, 0x24));
        assert_eq!(
            t.palette_heatmap[4],
            Color::Rgb(0xff, 0xc6, 0x6b),
            "amber dawn"
        );
    }

    #[test]
    fn no_chrome_token_is_reset_by_default() {
        // The Splash identity commits to every chrome token — leaking `Color::Reset` for
        // any of them would let the terminal's own foreground bleed through and break
        // the "splash owns the surface" look. Users who want terminal-native chrome
        // revert individual slots via `[theme] <name> = "reset"`; the defaults never do.
        let t = Theme::default();
        assert_ne!(t.bg, Color::Reset);
        assert_ne!(t.bg_subtle, Color::Reset);
        assert_ne!(t.text, Color::Reset);
        assert_ne!(t.panel_border, Color::Reset);
        assert_ne!(t.panel_title, Color::Reset);
    }

    #[test]
    fn from_config_overrides_only_provided_fields() {
        let cfg = ThemeConfig {
            status_ok: Some(Color::Blue),
            ..Default::default()
        };
        let t = Theme::from_config(&cfg);
        assert_eq!(t.status_ok, Color::Blue);
        // Untouched field retains its default.
        // status_error stays at the signature default because the override only touched ok.
        assert_eq!(t.status_error, Theme::default().status_error);
    }

    #[test]
    fn theme_config_parses_named_and_hex() {
        let toml = r##"
status_ok = "cyan"
status_error = "#ff8800"
palette_series = ["red", "#00ff00", "blue"]
"##;
        let cfg: ThemeConfig = toml::from_str(toml).unwrap();
        let t = Theme::from_config(&cfg);
        assert_eq!(t.status_ok, Color::Cyan);
        assert_eq!(t.status_error, Color::Rgb(0xff, 0x88, 0x00));
        assert_eq!(t.palette_series.len(), 3);
    }

    #[test]
    fn series_color_wraps_around() {
        let t = Theme::default();
        assert_eq!(t.series_color(0), t.series_color(t.palette_series.len()));
    }

    #[test]
    fn series_color_falls_back_on_empty_palette() {
        let t = Theme {
            palette_series: vec![],
            ..Theme::default()
        };
        assert_eq!(t.series_color(0), Color::Cyan);
    }

    #[test]
    fn heatmap_level_saturates_at_top() {
        let t = Theme::default();
        assert_eq!(t.heatmap_level(99), t.heatmap_level(4));
    }

    #[test]
    fn preset_with_overlay_layers_correctly() {
        // Regression guard for the two-step resolution: preset picks the base, then
        // individual fields overlay. If someone reorders those steps later the preset's
        // value would win, which silently ignores the user's override.
        let cfg = ThemeConfig {
            preset: Some("nord".into()),
            status_ok: Some(Color::Magenta),
            ..Default::default()
        };
        let t = Theme::from_config(&cfg);
        assert_eq!(t.status_ok, Color::Magenta, "override must beat preset");
        // Untouched tokens come from the preset, not the built-in default.
        assert_ne!(t.status_error, Theme::default().status_error);
    }

    #[test]
    fn unknown_preset_falls_back_to_default() {
        let cfg = ThemeConfig {
            preset: Some("bogus".into()),
            ..Default::default()
        };
        let t = Theme::from_config(&cfg);
        assert_eq!(t.status_ok, Theme::default().status_ok);
    }

    #[test]
    fn empty_series_override_keeps_default() {
        // An empty array in TOML shouldn't nuke the palette — that would silently break the
        // charts. Keep the built-in and let the user explicitly disable by writing a single-
        // element array if they really want that.
        let cfg = ThemeConfig {
            palette_series: Some(vec![]),
            ..Default::default()
        };
        let t = Theme::from_config(&cfg);
        assert_eq!(t.palette_series.len(), 10);
    }
}
