#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::layout::{BorderStyle, Child, Flex, Layout};
use crate::render::RenderSpec;

const DEFAULT_CONFIG: &str = include_str!("default_config.toml");

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default, rename = "widget")]
    pub widgets: Vec<WidgetConfig>,
    #[serde(default, rename = "row")]
    pub rows: Vec<RowConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct General {
    #[serde(default)]
    pub wait_for_fresh: bool,
    /// Inline viewport height in rows. `None` uses the built-in default. Configs that ship more
    /// widgets than fit in the default bump this to make room.
    #[serde(default)]
    pub height: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WidgetConfig {
    pub id: String,
    pub fetcher: String,
    /// Renderer selection. `render = "simple"` (short form) or
    /// `render = { type = "ascii_art", pixel_size = "quadrant" }` (full form with options).
    /// Absent = pick the default renderer for the fetcher's shape.
    #[serde(default)]
    pub render: Option<RenderSpec>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub refresh_interval: Option<u64>,
    /// read_store: file format — "json", "toml", or "text". Other fetchers ignore it.
    #[serde(default)]
    pub file_format: Option<String>,
    /// Fetcher-specific options as a TOML sub-table (`[widget.options]`). Each fetcher
    /// deserializes the keys it cares about; unknown keys are ignored so upgrading the
    /// fetcher never invalidates old configs.
    #[serde(default)]
    pub options: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RowConfig {
    #[serde(default)]
    pub height: Option<SizeSpec>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub border: Option<BorderSpec>,
    /// How children are placed along this row's horizontal axis when they don't fill the row.
    /// `center` is the obvious use case: a narrower widget parked in the middle of its row.
    #[serde(default)]
    pub flex: Option<FlexSpec>,
    #[serde(default, rename = "child")]
    pub children: Vec<ChildConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlexSpec {
    Legacy,
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChildConfig {
    pub widget: String,
    #[serde(default)]
    pub width: Option<SizeSpec>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub border: Option<BorderSpec>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeSpec {
    Fill(u16),
    Length(u16),
    Min(u16),
    Max(u16),
    Percentage(u16),
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BorderSpec {
    /// No chrome at all — widget fills its full cell, no title drawn. Useful for hero
    /// elements like a big clock that should own its slot without framing.
    None,
    Plain,
    Rounded,
    Thick,
    Double,
}

impl Config {
    pub fn default_baked() -> Self {
        toml::from_str(DEFAULT_CONFIG).expect("built-in default config must parse")
    }

    pub fn load_or_default(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default_baked()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn parse(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }

    pub fn to_layout(&self) -> Layout {
        Layout::rows(self.rows.iter().map(to_row_child).collect())
    }

    /// Sum of row heights — what the inline viewport needs to be to fit every row without
    /// clipping the bottom. Non-Length sizes fall back to reasonable approximations
    /// (Min/Max use their value, Fill contributes a small default, Percentage is ignored
    /// since it can't resolve without a known viewport).
    pub fn computed_height(&self) -> u16 {
        self.rows
            .iter()
            .map(|r| row_height_estimate(r.height))
            .sum()
    }
}

fn row_height_estimate(size: Option<SizeSpec>) -> u16 {
    match size {
        Some(SizeSpec::Length(n)) | Some(SizeSpec::Min(n)) | Some(SizeSpec::Max(n)) => n,
        Some(SizeSpec::Fill(_)) => 3,
        Some(SizeSpec::Percentage(_)) => 0,
        None => 3,
    }
}

pub const LOCAL_CONFIG_CANDIDATES: &[&str] = &[".splashboard/config.toml", ".splashboard.toml"];

pub fn resolve_config_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    find_local(&cwd).or_else(default_global_path)
}

pub fn resolve_cwd_only_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    find_local_at(&cwd)
}

/// Walks up from the current directory looking for a project-local config. Used by the trust
/// commands so `splashboard trust` / `revoke` target the nearest `.splashboard.toml` the same
/// way `splashboard` itself would pick one up.
pub fn resolve_local_config_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    find_local(&cwd)
}

pub fn default_global_path() -> Option<PathBuf> {
    crate::paths::config_path()
}

fn find_local(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if let Some(path) = find_local_at(dir) {
            return Some(path);
        }
        current = dir.parent();
    }
    None
}

fn find_local_at(dir: &Path) -> Option<PathBuf> {
    LOCAL_CONFIG_CANDIDATES
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

fn to_row_child(row: &RowConfig) -> Child {
    let mut inner = Layout::cols(row.children.iter().map(to_col_child).collect());
    if let Some(f) = row.flex {
        inner = inner.flexed(to_flex(f));
    }
    let decorated = apply_panel(inner, row.title.as_deref(), row.border);
    make_child(row.height, decorated)
}

fn to_flex(f: FlexSpec) -> Flex {
    match f {
        FlexSpec::Legacy => Flex::Legacy,
        FlexSpec::Start => Flex::Start,
        FlexSpec::Center => Flex::Center,
        FlexSpec::End => Flex::End,
        FlexSpec::SpaceBetween => Flex::SpaceBetween,
        FlexSpec::SpaceAround => Flex::SpaceAround,
    }
}

fn to_col_child(c: &ChildConfig) -> Child {
    let leaf = Layout::widget(c.widget.clone());
    let decorated = apply_panel(leaf, c.title.as_deref(), c.border);
    make_child(c.width, decorated)
}

fn make_child(size: Option<SizeSpec>, layout: Layout) -> Child {
    match size {
        Some(SizeSpec::Fill(w)) => Child::fill(w, layout),
        Some(SizeSpec::Length(n)) => Child::length(n, layout),
        Some(SizeSpec::Min(n)) => Child::min(n, layout),
        Some(SizeSpec::Max(n)) => Child::max(n, layout),
        Some(SizeSpec::Percentage(p)) => Child::percentage(p, layout),
        None => Child::fill(1, layout),
    }
}

fn apply_panel(layout: Layout, title: Option<&str>, border: Option<BorderSpec>) -> Layout {
    // Chrome is opt-in: no border, no panel — even if a title was set. A bare title has no
    // border to render on, so dropping it cleanly is better than pretending. Users who want
    // the framed-with-label look set `border = "rounded"` (or plain/thick/double) explicitly
    // on the widgets that deserve it (charts, tables), keeping hero widgets (clock, greeting)
    // chromeless by default.
    let Some(style) = border.and_then(to_border_style) else {
        return layout;
    };
    let l = match title {
        Some(t) => layout.titled(t),
        None => layout,
    };
    l.bordered(style)
}

fn to_border_style(b: BorderSpec) -> Option<BorderStyle> {
    match b {
        BorderSpec::None => None,
        BorderSpec::Plain => Some(BorderStyle::Plain),
        BorderSpec::Rounded => Some(BorderStyle::Rounded),
        BorderSpec::Thick => Some(BorderStyle::Thick),
        BorderSpec::Double => Some(BorderStyle::Double),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_baked_config_parses() {
        let _ = Config::default_baked();
    }

    #[test]
    fn default_baked_has_expected_widgets() {
        let c = Config::default_baked();
        let ids: Vec<&str> = c.widgets.iter().map(|w| w.id.as_str()).collect();
        assert!(ids.contains(&"greeting"));
        assert!(ids.contains(&"clock"));
        assert!(ids.contains(&"prs"));
    }

    #[test]
    fn minimal_config_parses() {
        let toml = r#"
[[widget]]
id = "x"
fetcher = "static"
render = "text"

[[row]]
height = { length = 3 }
[[row.child]]
widget = "x"
"#;
        let c = Config::parse(toml).unwrap();
        assert_eq!(c.widgets.len(), 1);
        assert_eq!(c.rows.len(), 1);
        assert_eq!(c.rows[0].children.len(), 1);
    }

    #[test]
    fn size_spec_accepts_variants() {
        let toml = r#"
[[widget]]
id = "x"
fetcher = "f"
render = "text"

[[row]]
height = { fill = 2 }
[[row.child]]
widget = "x"
width = { percentage = 50 }

[[row]]
height = { min = 6 }
[[row.child]]
widget = "x"
width = { max = 30 }
"#;
        let c = Config::parse(toml).unwrap();
        assert!(matches!(c.rows[0].height, Some(SizeSpec::Fill(2))));
        assert!(matches!(
            c.rows[0].children[0].width,
            Some(SizeSpec::Percentage(50))
        ));
        assert!(matches!(c.rows[1].height, Some(SizeSpec::Min(6))));
        assert!(matches!(
            c.rows[1].children[0].width,
            Some(SizeSpec::Max(30))
        ));
    }

    #[test]
    fn border_spec_parses_all_styles() {
        for name in ["plain", "rounded", "thick", "double"] {
            let toml = format!(
                r#"
[[widget]]
id = "x"
fetcher = "f"
render = "text"

[[row]]
border = "{name}"
[[row.child]]
widget = "x"
"#
            );
            Config::parse(&toml).unwrap();
        }
    }

    #[test]
    fn invalid_toml_returns_error() {
        let r = Config::parse("this is not [valid toml");
        assert!(r.is_err());
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let path = Path::new("/does/not/exist.toml");
        let c = Config::load_or_default(path).unwrap();
        assert!(!c.widgets.is_empty());
    }

    #[test]
    fn find_local_picks_up_flat_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".splashboard.toml");
        std::fs::write(&file, "").unwrap();
        assert_eq!(find_local(dir.path()).unwrap(), file);
    }

    #[test]
    fn find_local_picks_up_directory_form() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join(".splashboard");
        std::fs::create_dir(&subdir).unwrap();
        let file = subdir.join("config.toml");
        std::fs::write(&file, "").unwrap();
        assert_eq!(find_local(dir.path()).unwrap(), file);
    }

    #[test]
    fn find_local_prefers_directory_form_when_both_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".splashboard")).unwrap();
        let dir_file = dir.path().join(".splashboard/config.toml");
        std::fs::write(&dir_file, "").unwrap();
        std::fs::write(dir.path().join(".splashboard.toml"), "").unwrap();
        assert_eq!(find_local(dir.path()).unwrap(), dir_file);
    }

    #[test]
    fn find_local_walks_up_to_ancestor() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(".splashboard.toml");
        std::fs::write(&file, "").unwrap();
        let nested = root.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_local(&nested).unwrap(), file);
    }

    #[test]
    fn find_local_returns_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_local(dir.path()).is_none());
    }

    #[test]
    fn find_local_at_does_not_walk_up() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".splashboard.toml"), "").unwrap();
        let nested = root.path().join("sub");
        std::fs::create_dir(&nested).unwrap();
        assert!(find_local_at(&nested).is_none());
        assert!(find_local(&nested).is_some());
    }

    #[test]
    fn to_layout_builds_nested_rows_of_cols() {
        let c = Config::default_baked();
        let layout = c.to_layout();
        match layout {
            Layout::Stack { children, .. } => {
                assert!(!children.is_empty());
            }
            _ => panic!("expected Stack at root"),
        }
    }
}
