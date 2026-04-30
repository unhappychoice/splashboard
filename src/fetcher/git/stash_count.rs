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
    use std::future::Future;

    use super::super::test_support::{commit, make_repo, stash};
    use super::*;

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn ctx(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            shape,
            ..FetchContext::default()
        }
    }

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        let fetcher = GitStashCount;
        let text_key = fetcher.cache_key(&ctx(Some(Shape::Text)));
        let entries_key = fetcher.cache_key(&ctx(Some(Shape::Entries)));

        assert_eq!(fetcher.name(), "git_stash_count");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("stash reflog"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Text);
        assert!(text_key.starts_with("git_stash_count-"));
        assert_ne!(text_key, entries_key);
        assert_eq!(
            fetcher.sample_body(Shape::Text),
            Some(samples::text("3 stashes"))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Entries),
            Some(samples::entries(&[("stashes", "3")]))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Badge),
            Some(samples::badge(Status::Warn, "3 stashes"))
        );
        assert!(fetcher.sample_body(Shape::TextBlock).is_none());
    }

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
        assert_eq!(
            render_body(stash_count(&repo).unwrap(), Shape::Text),
            text_body("")
        );
    }

    #[test]
    fn text_shape_uses_singular_label_for_one_stash() {
        assert_eq!(render_body(1, Shape::Text), text_body("1 stash"));
    }

    #[test]
    fn badge_shape_status_reflects_count() {
        assert_eq!(
            render_body(0, Shape::Badge),
            Body::Badge(BadgeData {
                status: Status::Ok,
                label: "no stashes".into(),
            })
        );
        assert_eq!(
            render_body(2, Shape::Badge),
            Body::Badge(BadgeData {
                status: Status::Warn,
                label: "2 stashes".into(),
            })
        );
    }

    #[test]
    fn entries_shape_always_shows_count() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        assert_eq!(
            render_body(stash_count(&repo).unwrap(), Shape::Entries),
            Body::Entries(EntriesData {
                items: vec![Entry {
                    key: "stashes".into(),
                    value: Some("0".into()),
                    status: None,
                }],
            })
        );
    }

    #[test]
    fn fetch_uses_current_repo_for_default_and_explicit_shapes() {
        let fetcher = GitStashCount;
        let repo = open_repo().unwrap();
        let count = stash_count(&repo).unwrap();

        let default_payload = run_async(fetcher.fetch(&ctx(None))).unwrap();
        let entries_payload = run_async(fetcher.fetch(&ctx(Some(Shape::Entries)))).unwrap();
        let badge_payload = run_async(fetcher.fetch(&ctx(Some(Shape::Badge)))).unwrap();

        assert_eq!(default_payload, payload(render_body(count, Shape::Text)));
        assert_eq!(entries_payload, payload(render_body(count, Shape::Entries)));
        assert_eq!(badge_payload, payload(render_body(count, Shape::Badge)));
    }
}
