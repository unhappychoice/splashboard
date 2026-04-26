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
    use super::*;

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
