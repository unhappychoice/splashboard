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
    fn pretty_name_keeps_empty_underscore_segments_as_blank_words() {
        // A leading / trailing `_` yields an empty `split('_')` segment whose `chars().next()`
        // is `None`, exercising the `None => String::new()` arm rather than the capitalize path.
        assert_eq!(pretty_name("foo_"), "Foo ");
        assert_eq!(pretty_name("_bar"), " Bar");
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

    #[test]
    fn theme_for_falls_back_to_default_for_unknown_slug() {
        // `presets::by_name` misses → `unwrap_or_default()` materialises the default theme
        // rather than panicking, so a stale slug in the gallery still renders something.
        let _ = theme_for("not-a-real-theme");
    }

    fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "splashboard-themes-{tag}-{unique}-{}",
            std::process::id()
        ))
    }

    /// A minimal one-widget dashboard written to a temp file — enough for
    /// `render_config_html_with_theme` to parse, lay out, and snapshot without network or disk
    /// fetchers. Returns the path so the caller can pass it to `render_one` as the demo config.
    fn write_demo_dashboard(tag: &str) -> std::path::PathBuf {
        let path = unique_temp_dir(tag).with_extension("toml");
        fs::write(
            &path,
            r#"
[[widget]]
id = "hello"
fetcher = "basic_static"
format = "Hi"
render = "list_plain"

[[row]]
height = { length = 1 }
  [[row.child]]
  widget = "hello"
"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn render_one_writes_theme_html_and_returns_entry() {
        let demo = write_demo_dashboard("render-ok");
        let out_dir = unique_temp_dir("render-ok-out");
        fs::create_dir_all(&out_dir).unwrap();

        let entry = render_one("nord", &demo, &out_dir, 20, 6).unwrap();
        assert_eq!(entry.slug, "nord");
        assert_eq!(entry.name, "Nord");
        assert!(matches!(entry.kind, ThemeKind::Dark));

        let html = fs::read_to_string(out_dir.join("nord.html")).unwrap();
        assert!(html.starts_with("<pre"), "unexpected html: {html}");

        let _ = fs::remove_file(&demo);
        let _ = fs::remove_dir_all(&out_dir);
    }

    #[test]
    fn render_one_surfaces_render_failure_from_missing_demo_config() {
        // A demo path that doesn't exist makes `render_config_html_with_theme`'s `read_to_string`
        // fail; `render_one` should propagate that error rather than writing a stray html file.
        let missing = unique_temp_dir("render-missing").with_extension("toml");
        let out_dir = unique_temp_dir("render-missing-out");
        fs::create_dir_all(&out_dir).unwrap();

        let err = render_one("nord", &missing, &out_dir, 20, 6).unwrap_err();
        assert!(err.to_string().contains("read"), "unexpected error: {err}");

        let _ = fs::remove_dir_all(&out_dir);
    }

    #[test]
    fn render_one_surfaces_write_failure_when_output_path_is_a_directory() {
        // Pre-create `nord.html` as a directory so `fs::write` cannot replace it — `render_one`
        // should surface the failure with its `write` context after a successful render.
        let demo = write_demo_dashboard("render-write-fail");
        let out_dir = unique_temp_dir("render-write-fail-out");
        fs::create_dir_all(out_dir.join("nord.html")).unwrap();

        let err = render_one("nord", &demo, &out_dir, 20, 6).unwrap_err();
        assert!(err.to_string().contains("write"), "unexpected error: {err}");

        let _ = fs::remove_file(&demo);
        let _ = fs::remove_dir_all(&out_dir);
    }
}
