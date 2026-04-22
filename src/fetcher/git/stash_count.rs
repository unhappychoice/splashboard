use async_trait::async_trait;

use crate::payload::{Body, EntriesData, Entry, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, lines_body, open_repo, payload, repo_cache_key};

const SHAPES: &[Shape] = &[Shape::Lines, Shape::Entries];

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
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let count = stash_count(&repo)?;
        Ok(payload(render_body(
            count,
            ctx.shape.unwrap_or(Shape::Lines),
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
        _ => {
            if count == 0 {
                lines_body(Vec::new())
            } else {
                lines_body(vec![format!(
                    "{count} stash{}",
                    if count == 1 { "" } else { "es" }
                )])
            }
        }
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
    fn lines_shape_is_empty_when_none() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        let body = render_body(stash_count(&repo).unwrap(), Shape::Lines);
        match body {
            Body::Lines(d) => assert!(d.lines.is_empty()),
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
