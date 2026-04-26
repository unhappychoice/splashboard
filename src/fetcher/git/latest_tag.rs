use async_trait::async_trait;

use crate::payload::{Body, EntriesData, Entry, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[Shape::Text, Shape::Entries];

/// Most recent annotated-or-lightweight tag by committer time of the peeled commit. `Text` emits
/// just the tag name; `Entries` adds the target short hash and the commit time as ISO date so a
/// two-line "latest release" widget has somewhere to put the date.
pub struct GitLatestTag;

#[async_trait]
impl Fetcher for GitLatestTag {
    fn name(&self) -> &str {
        "git_latest_tag"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Most recent git tag by committer time, suitable as a \"latest release\" line. `Text` shows just the tag name; `Entries` adds the short commit hash and ISO date."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("v1.2.3"),
            Shape::Entries => samples::entries(&[
                ("tag", "v1.2.3"),
                ("short", "a1b2c3d"),
                ("date", "2026-04-14"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        build(&repo, ctx.shape.unwrap_or(Shape::Text))
    }
}

fn build(repo: &gix::Repository, shape: Shape) -> Result<Payload, FetchError> {
    let Some(tag) = latest_tag(repo)? else {
        return Ok(payload(text_body("")));
    };
    let body = match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("tag", &tag.name),
                entry("commit", &tag.short_hash),
                entry("date", &tag.iso_date),
            ],
        }),
        _ => text_body(tag.name),
    };
    Ok(payload(body))
}

struct TagInfo {
    name: String,
    short_hash: String,
    iso_date: String,
}

fn latest_tag(repo: &gix::Repository) -> Result<Option<TagInfo>, FetchError> {
    let refs = repo.references().map_err(fail)?;
    let tags = refs.tags().map_err(fail)?;
    let mut best: Option<(TagInfo, i64)> = None;
    for tag in tags {
        let mut tag = tag.map_err(fail)?;
        let name = tag.name().shorten().to_string();
        let id = tag.peel_to_id().map_err(fail)?;
        let short_hash = id.shorten().map_err(fail)?.to_string();
        let Ok(commit) = id.object().map_err(fail)?.try_into_commit() else {
            continue;
        };
        let time = commit.time().map_err(fail)?.seconds;
        let info = TagInfo {
            name,
            short_hash,
            iso_date: iso_date(time),
        };
        match best {
            Some((_, t)) if t >= time => {}
            _ => best = Some((info, time)),
        }
    }
    Ok(best.map(|(info, _)| info))
}

fn iso_date(unix_seconds: i64) -> String {
    use chrono::{DateTime, Utc};
    DateTime::<Utc>::from_timestamp(unix_seconds, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::make_repo;
    use super::*;

    #[test]
    fn returns_empty_text_when_no_tags() {
        let (_tmp, repo) = make_repo();
        let p = build(&repo, Shape::Text).unwrap();
        match p.body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn text_shape_emits_tag_name() {
        let (_tmp, repo) = make_repo();
        super::super::test_support::commit(&repo, "initial");
        super::super::test_support::tag(&repo, "v0.1.0");
        let p = build(&repo, Shape::Text).unwrap();
        match p.body {
            Body::Text(d) => assert_eq!(d.value, "v0.1.0"),
            _ => panic!(),
        }
    }

    #[test]
    fn entries_shape_has_tag_commit_date() {
        let (_tmp, repo) = make_repo();
        super::super::test_support::commit(&repo, "initial");
        super::super::test_support::tag(&repo, "v1.2.3");
        let p = build(&repo, Shape::Entries).unwrap();
        match p.body {
            Body::Entries(d) => {
                let keys: Vec<_> = d.items.iter().map(|e| e.key.as_str()).collect();
                assert_eq!(keys, ["tag", "commit", "date"]);
                assert_eq!(d.items[0].value.as_deref(), Some("v1.2.3"));
                assert_eq!(d.items[1].value.as_deref().unwrap().len(), 7);
                assert!(d.items[2].value.as_deref().unwrap().starts_with("20"));
            }
            _ => panic!(),
        }
    }
}
