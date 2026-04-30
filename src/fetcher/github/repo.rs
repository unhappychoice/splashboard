//! `github_repo` — identity triple `{ slug, description, license }` sourced from the GitHub
//! REST API `/repos/{o}/{n}`. Entries shape feeds `grid_table` (inline or rows); Text joins
//! the non-empty fields with `·` for a compact subtitle.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::payload::{Body, EntriesData, Entry, Payload, TextBlockData, TextData};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::resolve_repo;

pub struct GithubRepo;

#[async_trait]
impl Fetcher for GithubRepo {
    fn name(&self) -> &str {
        "github_repo"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Repo identity for the current directory: slug, description, and SPDX license. Use `github_repo_stars` instead for the social counters (stars, forks, watchers)."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Entries, Shape::TextBlock, Shape::Text]
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, _ctx: &FetchContext) -> String {
        cwd_scoped(self.name())
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => Body::Entries(EntriesData {
                items: vec![
                    entry("slug", "unhappychoice/splashboard"),
                    entry("description", "terminal splash renderer"),
                    entry("license", "ISC"),
                ],
            }),
            Shape::TextBlock => Body::TextBlock(TextBlockData {
                lines: vec![
                    "unhappychoice/splashboard".into(),
                    "terminal splash renderer".into(),
                    "ISC".into(),
                ],
            }),
            Shape::Text => Body::Text(TextData {
                value: "unhappychoice/splashboard · terminal splash renderer · ISC".into(),
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let slug = resolve_repo(None)?;
        let info: RepoInfo = rest_get(&format!("/repos/{}/{}", slug.owner, slug.name)).await?;
        let meta = Metadata {
            slug: Some(format!("{}/{}", slug.owner, slug.name)),
            description: info.description.filter(|s| !s.is_empty()),
            license: info.license.and_then(|l| l.spdx_id),
        };
        let body = match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::Text => Body::Text(TextData {
                value: meta.as_text(),
            }),
            Shape::TextBlock => Body::TextBlock(TextBlockData {
                lines: meta.as_lines(),
            }),
            _ => Body::Entries(EntriesData {
                items: meta.as_entries(),
            }),
        };
        Ok(payload(body))
    }
}

#[derive(Debug, Deserialize)]
struct RepoInfo {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    license: Option<LicenseInfo>,
}

#[derive(Debug, Deserialize)]
struct LicenseInfo {
    #[serde(default)]
    spdx_id: Option<String>,
}

#[derive(Default)]
struct Metadata {
    slug: Option<String>,
    description: Option<String>,
    license: Option<String>,
}

impl Metadata {
    fn as_entries(&self) -> Vec<Entry> {
        [
            ("slug", self.slug.as_deref()),
            ("description", self.description.as_deref()),
            ("license", self.license.as_deref()),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.map(|val| entry(k, val)))
        .collect()
    }
    fn as_text(&self) -> String {
        [
            self.slug.as_deref(),
            self.description.as_deref(),
            self.license.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
    }
    fn as_lines(&self) -> Vec<String> {
        [
            self.slug.as_deref(),
            self.description.as_deref(),
            self.license.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::to_owned)
        .collect()
    }
}

fn cwd_scoped(name: &str) -> String {
    let cwd: PathBuf = std::env::current_dir().unwrap_or_default();
    let digest = Sha256::digest(cwd.to_string_lossy().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    match value {
                        Some(value) => unsafe { std::env::set_var(key, value) },
                        None => unsafe { std::env::remove_var(key) },
                    }
                    (*key, previous)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.restore.iter().for_each(|(key, value)| match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            });
        }
    }

    #[test]
    fn metadata_as_text_joins_non_empty_fields_with_middot() {
        let m = Metadata {
            slug: Some("foo/bar".into()),
            description: Some("a thing".into()),
            license: Some("ISC".into()),
        };
        assert_eq!(m.as_text(), "foo/bar · a thing · ISC");
    }

    #[test]
    fn metadata_as_text_skips_missing_pieces() {
        let m = Metadata {
            slug: Some("foo/bar".into()),
            description: None,
            license: Some("ISC".into()),
        };
        assert_eq!(m.as_text(), "foo/bar · ISC");
    }

    #[test]
    fn metadata_as_entries_filters_out_empty_keys() {
        let m = Metadata {
            slug: Some("foo/bar".into()),
            description: None,
            license: Some("ISC".into()),
        };
        let entries = m.as_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "slug");
        assert_eq!(entries[1].key, "license");
    }

    #[test]
    fn sample_entries_has_three_rows() {
        let sample = GithubRepo.sample_body(Shape::Entries).unwrap();
        match sample {
            Body::Entries(d) => assert_eq!(d.items.len(), 3),
            _ => panic!("expected entries sample"),
        }
    }

    #[test]
    fn metadata_as_lines_preserves_visible_order() {
        let m = Metadata {
            slug: Some("foo/bar".into()),
            description: Some("a thing".into()),
            license: Some("ISC".into()),
        };
        assert_eq!(m.as_lines(), vec!["foo/bar", "a thing", "ISC"]);
    }

    #[test]
    fn fetcher_metadata_cache_key_and_samples_match_contract() {
        let fetcher = GithubRepo;
        let default_key = fetcher.cache_key(&FetchContext::default());
        let shaped_key = fetcher.cache_key(&FetchContext {
            format: Some("markdown".into()),
            timeout: Duration::from_secs(1),
            shape: Some(Shape::Text),
            ..Default::default()
        });

        assert_eq!(fetcher.name(), "github_repo");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("SPDX license"));
        assert_eq!(
            fetcher.shapes(),
            &[Shape::Entries, Shape::TextBlock, Shape::Text]
        );
        assert_eq!(fetcher.default_shape(), Shape::Entries);
        assert_eq!(default_key, shaped_key);
        assert!(default_key.starts_with("github_repo-"));

        let Some(Body::TextBlock(block)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(
            block.lines,
            vec![
                "unhappychoice/splashboard",
                "terminal splash renderer",
                "ISC"
            ]
        );

        let Some(Body::Text(text)) = fetcher.sample_body(Shape::Text) else {
            panic!("expected text sample");
        };
        assert_eq!(
            text.value,
            "unhappychoice/splashboard · terminal splash renderer · ISC"
        );
        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn entry_and_payload_helpers_preserve_body_shape() {
        let row = entry("slug", "foo/bar");
        assert_eq!(row.key, "slug");
        assert_eq!(row.value.as_deref(), Some("foo/bar"));
        assert!(row.status.is_none());

        let wrapped = payload(Body::Text(TextData {
            value: "foo/bar".into(),
        }));
        assert!(wrapped.icon.is_none());
        assert!(wrapped.status.is_none());
        assert!(wrapped.format.is_none());
        assert!(matches!(wrapped.body, Body::Text(_)));
    }

    #[tokio::test]
    async fn fetch_without_token_returns_auth_error_after_repo_resolution() {
        let _guard = EnvGuard::set(&[("GH_TOKEN", None), ("GITHUB_TOKEN", None)]);
        let err = GithubRepo
            .fetch(&FetchContext {
                timeout: Duration::from_secs(1),
                shape: Some(Shape::Text),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            FetchError::Failed(message) if message.contains("GH_TOKEN / GITHUB_TOKEN not set")
        ));
    }

    #[test]
    fn repo_info_deserializes_nested_license_spdx() {
        let raw = r#"{"description":"a tool","license":{"spdx_id":"ISC"}}"#;
        let info: RepoInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.description.as_deref(), Some("a tool"));
        assert_eq!(info.license.unwrap().spdx_id.as_deref(), Some("ISC"));
    }

    #[test]
    fn repo_info_tolerates_missing_license() {
        let raw = r#"{"description":"x"}"#;
        let info: RepoInfo = serde_json::from_str(raw).unwrap();
        assert!(info.license.is_none());
    }
}
