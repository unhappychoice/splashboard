use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::paths;
use crate::payload::{Body, ImageData, Payload};
use crate::render::Shape;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::languages;
use super::logo_assets;
use super::scan::for_each_tracked_path;

const SHAPES: &[Shape] = &[Shape::Image];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "language",
    type_hint: "string",
    required: false,
    default: Some("auto-detected from the dominant language in the repo"),
    description: "Override the detected language. Accepts the labels emitted by the shared `code_*` classifier (`Rust`, `Go`, `Python`, …); unknown values fall back to the generic `</>` glyph.",
}];

/// Picks the dominant programming language across the discovered git repo's tracked files and
/// renders a bundled Devicon PNG for it. Counts files (not lines) per language — cheap, no
/// blob reads, and "what kind of repo is this" is more honest by file mix than line totals.
/// Languages without a dedicated asset fall back to a generic `</>` glyph so the
/// `project_codebase` hero never renders empty.
pub struct CodeLanguageLogo;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    language: Option<String>,
}

#[async_trait]
impl Fetcher for CodeLanguageLogo {
    fn name(&self) -> &str {
        "code_language_logo"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Detects the dominant language across the repo's tracked files and emits a bundled Devicon PNG (covers ~35 languages; falls back to a generic `</>` glyph). Pair with the `media_image` renderer for a `project_codebase`-style hero that scales to whatever cell the layout assigns."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Mix the `language` option into the key so an explicit override doesn't collide with
        // the auto-detected entry for the same repo.
        let base = repo_cache_key(self.name(), ctx);
        let opts = ctx
            .options
            .as_ref()
            .map(toml::Value::to_string)
            .unwrap_or_default();
        if opts.is_empty() {
            return base;
        }
        let digest = Sha256::digest(opts.as_bytes());
        let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("{base}-{hex}")
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let language = match opts.language {
            Some(label) => Some(label),
            None => detect_dominant_language()?.map(String::from),
        };
        let (slug, bytes) = logo_assets::asset_for(language.as_deref());
        let path = ensure_extracted(slug, bytes)?;
        Ok(payload(Body::Image(ImageData {
            path: path.to_string_lossy().into_owned(),
        })))
    }
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    match raw {
        None => Ok(Options::default()),
        Some(value) => value
            .clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}"))),
    }
}

fn detect_dominant_language() -> Result<Option<&'static str>, FetchError> {
    let repo = open_repo()?;
    detect_dominant_in(&repo)
}

fn detect_dominant_in(repo: &gix::Repository) -> Result<Option<&'static str>, FetchError> {
    let mut counts: HashMap<&'static str, u32> = HashMap::new();
    for_each_tracked_path(repo, |path| {
        if let Some(lang) = languages::classify(path) {
            *counts.entry(lang).or_insert(0) += 1;
        }
    })?;
    Ok(counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(lang, _)| lang))
}

/// Materialises the bundled bytes onto disk so `media_image` (which only takes a path) can
/// open them. Stable per-slug filename under the cache dir means repeat invocations reuse the
/// same file and ratatui-image's protocol can warm-cache decoded frames.
fn ensure_extracted(slug: &str, bytes: &[u8]) -> Result<PathBuf, FetchError> {
    let dir = logos_cache_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| FetchError::Failed(format!("create logos cache dir: {e}")))?;
    let path = dir.join(format!("{slug}.png"));
    if !path.exists() {
        std::fs::write(&path, bytes)
            .map_err(|e| FetchError::Failed(format!("write logo {slug}: {e}")))?;
    }
    Ok(path)
}

fn logos_cache_dir() -> Result<PathBuf, FetchError> {
    paths::cache_dir()
        .map(|d| d.join("logos"))
        .ok_or_else(|| FetchError::Failed("no SPLASHBOARD_HOME / HOME".into()))
}

#[cfg(test)]
mod tests {
    use super::super::super::git::test_support::{commit_touching, make_repo};
    use super::*;
    use crate::paths;

    fn with_home<F: FnOnce()>(f: F) {
        let _lock = paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        unsafe {
            std::env::set_var("SPLASHBOARD_HOME", tmp.path());
        }
        f();
        unsafe {
            match previous {
                Some(v) => std::env::set_var("SPLASHBOARD_HOME", v),
                None => std::env::remove_var("SPLASHBOARD_HOME"),
            }
        }
    }

    #[test]
    fn dominant_language_picks_max_file_count() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/a.rs", "fn a() {}");
        commit_touching(&repo, "src/b.rs", "fn b() {}");
        commit_touching(&repo, "src/c.rs", "fn c() {}");
        commit_touching(&repo, "doc.md", "# hi");
        let dominant = detect_dominant_in(&repo).unwrap();
        assert_eq!(dominant, Some("Rust"));
    }

    #[test]
    fn dominant_language_returns_none_for_empty_repo() {
        let (_tmp, repo) = make_repo();
        assert!(detect_dominant_in(&repo).unwrap().is_none());
    }

    #[test]
    fn dominant_language_ignores_unclassified_paths() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.go", "package x");
        commit_touching(&repo, "config.weird", "noise");
        commit_touching(&repo, "another.weird", "more noise");
        // Two unknown files outweigh one Go file by raw count, but `classify` returns None so
        // they don't contribute — Go must still win.
        assert_eq!(detect_dominant_in(&repo).unwrap(), Some("Go"));
    }

    #[test]
    fn ensure_extracted_writes_bundled_bytes_once() {
        with_home(|| {
            let (slug, bytes) = logo_assets::asset_for(Some("Rust"));
            let path = ensure_extracted(slug, bytes).unwrap();
            assert!(path.exists());
            let written = std::fs::read(&path).unwrap();
            assert_eq!(written.len(), bytes.len());
            // Second call should be idempotent — same path, no rewrite needed.
            let path2 = ensure_extracted(slug, bytes).unwrap();
            assert_eq!(path, path2);
        });
    }

    #[test]
    fn parse_options_rejects_unknown_keys() {
        let bad: toml::Value = toml::from_str(r#"unknown = 1"#).unwrap();
        assert!(parse_options(Some(&bad)).is_err());
    }

    #[test]
    fn parse_options_accepts_language_override() {
        let v: toml::Value = toml::from_str(r#"language = "Python""#).unwrap();
        let opts = parse_options(Some(&v)).unwrap();
        assert_eq!(opts.language.as_deref(), Some("Python"));
    }

    #[test]
    fn cache_key_changes_with_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let k0 = CodeLanguageLogo.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"language = "Rust""#).unwrap());
        let k1 = CodeLanguageLogo.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"language = "Go""#).unwrap());
        let k2 = CodeLanguageLogo.cache_key(&ctx);
        assert_ne!(k0, k1);
        assert_ne!(k1, k2);
    }
}
