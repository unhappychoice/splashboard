use async_trait::async_trait;

use crate::payload::{Body, EntriesData, Entry, Payload};
use crate::render::Shape;
use crate::samples;
use crate::time as t;

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
                ("commit", "a1b2c3d"),
                ("date", "2026-04-14"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        build(
            &repo,
            ctx.shape.unwrap_or(Shape::Text),
            ctx.timezone.as_deref(),
            ctx.locale.as_deref(),
        )
    }
}

fn build(
    repo: &gix::Repository,
    shape: Shape,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> Result<Payload, FetchError> {
    let Some(tag) = latest_tag(repo, timezone, locale)? else {
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

fn latest_tag(
    repo: &gix::Repository,
    timezone: Option<&str>,
    locale: Option<&str>,
) -> Result<Option<TagInfo>, FetchError> {
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
            iso_date: iso_date(time, timezone, locale),
        };
        match best {
            Some((_, t)) if t >= time => {}
            _ => best = Some((info, time)),
        }
    }
    Ok(best.map(|(info, _)| info))
}

fn iso_date(unix_seconds: i64, timezone: Option<&str>, locale: Option<&str>) -> String {
    use chrono::{DateTime, Utc};
    let Some(utc) = DateTime::<Utc>::from_timestamp(unix_seconds, 0) else {
        return String::new();
    };
    let local = match t::parse_tz(timezone) {
        Some(tz) => utc.with_timezone(&tz).fixed_offset(),
        None => utc.with_timezone(&chrono::Local).fixed_offset(),
    };
    t::format_local(&local, "%Y-%m-%d", locale)
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
    use std::future::Future;
    use std::process::Command;

    use super::super::test_support::{commit, make_repo, tag};
    use super::*;

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn ctx(shape: Option<Shape>, timezone: Option<&str>) -> FetchContext {
        FetchContext {
            shape,
            timezone: timezone.map(str::to_string),
            ..FetchContext::default()
        }
    }

    fn dated_commit(repo: &gix::Repository, msg: &str, date: &str) {
        let path = repo.workdir().expect("workdir");
        let file = path.join("README.md");
        let prev = std::fs::read_to_string(&file).unwrap_or_default();
        std::fs::write(&file, format!("{prev}{msg}\n")).unwrap();
        let add = Command::new("git")
            .args(["add", "."])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(add.status.success(), "git add failed");
        let commit = Command::new("git")
            .args(["commit", "-q", "-m", msg])
            .current_dir(path)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(commit.status.success(), "git commit failed");
    }

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        let fetcher = GitLatestTag;
        let text_key = fetcher.cache_key(&ctx(Some(Shape::Text), None));
        let entries_key = fetcher.cache_key(&ctx(Some(Shape::Entries), None));

        assert_eq!(fetcher.name(), "git_latest_tag");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Most recent git tag"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Text);
        assert!(text_key.starts_with("git_latest_tag-"));
        assert_ne!(text_key, entries_key);
        assert_eq!(
            fetcher.sample_body(Shape::Text),
            Some(samples::text("v1.2.3"))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Entries),
            Some(samples::entries(&[
                ("tag", "v1.2.3"),
                ("commit", "a1b2c3d"),
                ("date", "2026-04-14"),
            ]))
        );
        assert!(fetcher.sample_body(Shape::Badge).is_none());
    }

    #[test]
    fn returns_empty_text_when_no_tags() {
        let (_tmp, repo) = make_repo();
        let p = build(&repo, Shape::Text, None, None).unwrap();
        match p.body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn text_shape_emits_tag_name() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        tag(&repo, "v0.1.0");
        let p = build(&repo, Shape::Text, None, None).unwrap();
        match p.body {
            Body::Text(d) => assert_eq!(d.value, "v0.1.0"),
            _ => panic!(),
        }
    }

    #[test]
    fn entries_shape_has_tag_commit_date() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        tag(&repo, "v1.2.3");
        let p = build(&repo, Shape::Entries, None, None).unwrap();
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

    #[test]
    fn latest_tag_prefers_newest_commit_time() {
        let (_tmp, repo) = make_repo();
        dated_commit(&repo, "oldest", "2026-01-01T00:00:00Z");
        tag(&repo, "c-oldest");
        dated_commit(&repo, "older", "2026-01-02T00:00:00Z");
        tag(&repo, "b-older");
        dated_commit(&repo, "newest", "2026-01-03T00:00:00Z");
        tag(&repo, "a-newest");

        let p = build(&repo, Shape::Entries, Some("UTC"), None).unwrap();

        assert!(matches!(
            p.body,
            Body::Entries(EntriesData { items })
                if items[0].value.as_deref() == Some("a-newest")
                    && items[2].value.as_deref() == Some("2026-01-03")
        ));
    }

    #[test]
    fn fetch_opens_cwd_repo_for_default_and_entries_shapes() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        tag(&repo, "v9.9.9");
        let workdir = repo.workdir().unwrap().to_path_buf();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workdir).unwrap();

        let text = run_async(GitLatestTag.fetch(&ctx(None, None)));
        let entries = run_async(GitLatestTag.fetch(&ctx(Some(Shape::Entries), None)));

        std::env::set_current_dir(prev_cwd).unwrap();

        let text = text.unwrap();
        let entries = entries.unwrap();
        assert!(matches!(text.body, Body::Text(d) if d.value == "v9.9.9"));
        assert!(matches!(
            entries.body,
            Body::Entries(EntriesData { items })
                if items.len() == 3
                    && items[0].key == "tag"
                    && items[0].value.as_deref() == Some("v9.9.9")
        ));
    }

    #[test]
    fn iso_date_handles_invalid_timestamp_and_explicit_timezone() {
        assert_eq!(iso_date(i64::MAX, Some("UTC"), None), "");
        assert_eq!(iso_date(0, Some("America/Los_Angeles"), None), "1969-12-31");
    }
}
