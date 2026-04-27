use async_trait::async_trait;

use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, Status};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[Shape::Text, Shape::Entries, Shape::Badge];

/// Number of entries in the stash reflog. A missing `refs/stash` ref or an absent reflog returns
/// zero — both are common (no stashes yet) and shouldn't surface as an error.
pub struct GitStashCount;

#[async_trait]
impl Fetcher for GitStashCount {
    fn name(&self) -> &str {
        "git_stash_count"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Number of entries in the stash reflog, as a quiet reminder of work parked aside. `Text` collapses to empty when there are zero stashes; `Entries` always reports the count; `Badge` shows the count as a pill (Ok at zero, Warn otherwise)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("3 stashes"),
            Shape::Entries => samples::entries(&[("stashes", "3")]),
            Shape::Badge => samples::badge(Status::Warn, "3 stashes"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let count = stash_count(&repo)?;
        Ok(payload(render_body(
            count,
            ctx.shape.unwrap_or(Shape::Text),
        )))
    }
}

fn stash_count(repo: &gix::Repository) -> Result<usize, FetchError> {
    let Some(stash_ref) = repo.try_find_reference("refs/stash").map_err(fail)? else {
        return Ok(0);
    };
    let mut platform = stash_ref.log_iter();
    match platform.all().map_err(fail)? {
        Some(iter) => Ok(iter.filter_map(Result::ok).count()),
        None => Ok(0),
    }
}

fn render_body(count: usize, shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![Entry {
                key: "stashes".into(),
                value: Some(count.to_string()),
                status: None,
            }],
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: if count == 0 { Status::Ok } else { Status::Warn },
            label: stash_label(count),
        }),
        _ => {
            if count == 0 {
                text_body("")
            } else {
                text_body(stash_label(count))
            }
        }
    }
}

fn stash_label(count: usize) -> String {
    match count {
        0 => "no stashes".into(),
        1 => "1 stash".into(),
        n => format!("{n} stashes"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{commit, make_repo, stash};
    use super::*;

    #[test]
    fn zero_when_no_stash() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        assert_eq!(stash_count(&repo).unwrap(), 0);
    }

    #[test]
    fn counts_single_stash() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        stash(&repo);
        assert_eq!(stash_count(&repo).unwrap(), 1);
    }

    #[test]
    fn text_shape_is_empty_when_none() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        let body = render_body(stash_count(&repo).unwrap(), Shape::Text);
        match body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn badge_shape_status_reflects_count() {
        let body = render_body(0, Shape::Badge);
        match body {
            Body::Badge(b) => {
                assert_eq!(b.status, Status::Ok);
                assert_eq!(b.label, "no stashes");
            }
            _ => panic!(),
        }
        let body = render_body(2, Shape::Badge);
        match body {
            Body::Badge(b) => {
                assert_eq!(b.status, Status::Warn);
                assert_eq!(b.label, "2 stashes");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn entries_shape_always_shows_count() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        let body = render_body(stash_count(&repo).unwrap(), Shape::Entries);
        match body {
            Body::Entries(d) => {
                assert_eq!(d.items[0].key, "stashes");
                assert_eq!(d.items[0].value.as_deref(), Some("0"));
            }
            _ => panic!(),
        }
    }
}
