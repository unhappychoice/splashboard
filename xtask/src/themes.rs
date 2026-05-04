//! Theme gallery — renders a single demo dashboard under each built-in theme preset and
//! emits a `_themes.json` index so `themes.mdx` can render the side-by-side comparison.
//!
//! The demo dashboard is `home_daily` because it exercises the most theme tokens at once:
//! calendar accents, status colours on gauges, the heatmap-ish almanac strip, panel
//! titles, and the series palette through chart widgets. Whatever theme is loaded, every
//! token shows up at least once so the comparison is honest.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use splashboard::theme::Theme;
use splashboard::theme::presets;

use crate::dashboard_snapshot;

/// Preset name → demo dashboard. `home_daily` covers the most tokens; if a future theme has
/// edge cases the demo doesn't surface, swap this out per-theme rather than per-page.
const DEMO_DASHBOARD: &str = "src/templates/home_daily.toml";

#[derive(Debug, Serialize)]
struct ThemeEntry {
    slug: String,
    name: String,
    kind: ThemeKind,
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ThemeKind {
    Dark,
    Light,
}

/// Light themes. Everything else is treated as dark — both for the index `kind` field and
/// for the visual grouping on `themes.mdx`. Keeping the list explicit means new dark themes
/// land in the right bucket by default.
const LIGHT_THEMES: &[&str] = &[
    "catppuccin_latte",
    "github_light",
    "gruvbox_light",
    "rose_pine_dawn",
    "solarized_light",
];

pub fn run(out_dir: &Path, width: u16, height: u16) -> Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    let demo = Path::new(DEMO_DASHBOARD);

    let entries = ordered_slugs()
        .iter()
        .map(|slug| render_one(slug, demo, out_dir, width, height))
        .collect::<Result<Vec<_>>>()?;

    let index_path = out_dir.join("_index.json");
    let index_json = serde_json::to_string_pretty(&entries).context("serialize themes index")?;
    fs::write(&index_path, index_json)
        .with_context(|| format!("write {}", index_path.display()))?;
    println!("wrote {}", index_path.display());
    Ok(())
}

fn render_one(
    slug: &str,
    demo: &Path,
    out_dir: &Path,
    width: u16,
    height: u16,
) -> Result<ThemeEntry> {
    let theme = theme_for(slug);
    let html = dashboard_snapshot::render_config_html_with_theme(demo, width, height, theme)?;
    let dest = out_dir.join(format!("{slug}.html"));
    fs::write(&dest, html).with_context(|| format!("write {}", dest.display()))?;
    println!("wrote {}", dest.display());
    Ok(ThemeEntry {
        slug: slug.to_string(),
        name: pretty_name(slug),
        kind: kind_for(slug),
    })
}

fn theme_for(slug: &str) -> Theme {
    if slug == "default" {
        Theme::default()
    } else {
        presets::by_name(slug).unwrap_or_default()
    }
}

fn kind_for(slug: &str) -> ThemeKind {
    if LIGHT_THEMES.contains(&slug) {
        ThemeKind::Light
    } else {
        ThemeKind::Dark
    }
}

/// `default` is the implicit setting (no `[theme] preset`), so it leads the gallery; the
/// rest follow `KNOWN`'s alphabetical order so siblings sort together.
fn ordered_slugs() -> Vec<&'static str> {
    let mut out = vec!["default"];
    out.extend(presets::KNOWN.iter().filter(|s| **s != "default").copied());
    out
}

/// `catppuccin_mocha` → "Catppuccin Mocha", `synthwave_84` → "Synthwave 84",
/// `default` → "Splash (default)". Cosmetic only — the slug is the source of truth.
/// `OVERRIDES` handles brand spellings the dumb capitalize-each-word rule gets wrong.
const OVERRIDES: &[(&str, &str)] = &[
    ("default", "Splash (default)"),
    ("github_dark", "GitHub Dark"),
    ("github_light", "GitHub Light"),
];

fn pretty_name(slug: &str) -> String {
    if let Some((_, name)) = OVERRIDES.iter().find(|(k, _)| *k == slug) {
        return (*name).to_string();
    }
    slug.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_name_capitalizes_each_underscore_segment() {
        assert_eq!(pretty_name("catppuccin_mocha"), "Catppuccin Mocha");
        assert_eq!(pretty_name("synthwave_84"), "Synthwave 84");
        assert_eq!(pretty_name("nord"), "Nord");
    }

    #[test]
    fn pretty_name_uses_brand_overrides_where_capitalize_is_wrong() {
        assert_eq!(pretty_name("default"), "Splash (default)");
        assert_eq!(pretty_name("github_dark"), "GitHub Dark");
        assert_eq!(pretty_name("github_light"), "GitHub Light");
    }

    #[test]
    fn ordered_slugs_lists_default_first_then_remaining_known_themes() {
        let order = ordered_slugs();
        assert_eq!(order[0], "default");
        assert_eq!(order.len(), presets::KNOWN.len());
        assert!(order.iter().skip(1).all(|s| *s != "default"));
    }

    #[test]
    fn kind_for_routes_known_light_themes_to_light_and_others_to_dark() {
        for slug in LIGHT_THEMES {
            assert!(matches!(kind_for(slug), ThemeKind::Light));
        }
        assert!(matches!(kind_for("nord"), ThemeKind::Dark));
        assert!(matches!(kind_for("default"), ThemeKind::Dark));
    }

    #[test]
    fn theme_for_resolves_default_and_known_presets() {
        let _ = theme_for("default");
        for slug in presets::KNOWN {
            let _ = theme_for(slug);
        }
    }
}
