use async_trait::async_trait;

use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, Status as PayloadStatus};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[Shape::Entries, Shape::Text, Shape::Badge];

/// Working tree snapshot: branch, dirty flag, ahead/behind vs upstream. Multi-shape so a user can
/// pick a key/value panel (`Entries`, default), a one-liner (`Text`), or a clean/dirty traffic
/// light (`Badge`). Detached HEAD shows the short commit hash in place of the branch.
pub struct GitStatus;

#[async_trait]
impl Fetcher for GitStatus {
    fn name(&self) -> &str {
        "git_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Working-tree snapshot: current branch, clean/dirty flag, and ahead/behind counts vs upstream. Choose `Entries` for a key/value panel, `Text` for a one-liner, or `Badge` for a clean/dirty traffic light."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("branch", "main"),
                ("ahead", "2"),
                ("modified", "3"),
                ("untracked", "1"),
            ]),
            Shape::Text => samples::text("main  +2 −0  ◆3 ?1"),
            Shape::Badge => samples::badge(PayloadStatus::Ok, "clean"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let info = read_status(&repo)?;
        Ok(payload(render_body(
            &info,
            ctx.shape.unwrap_or(Shape::Entries),
        )))
    }
}

struct StatusInfo {
    branch: String,
    detached: bool,
    head_short: String,
    dirty: bool,
    ahead: Option<usize>,
    behind: Option<usize>,
}

fn read_status(repo: &gix::Repository) -> Result<StatusInfo, FetchError> {
    let head_ref = repo.head_ref().map_err(fail)?;
    let detached = head_ref.is_none();
    let branch = head_ref
        .as_ref()
        .map(|r| r.name().shorten().to_string())
        .unwrap_or_else(|| "HEAD".into());
    let head_short = repo
        .head_id()
        .ok()
        .and_then(|id| id.shorten().ok().map(|p| p.to_string()))
        .unwrap_or_default();
    let dirty = repo.is_dirty().unwrap_or(false);
    let (ahead, behind) = match ahead_behind(repo, head_ref.as_ref()) {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    Ok(StatusInfo {
        branch,
        detached,
        head_short,
        dirty,
        ahead,
        behind,
    })
}

fn ahead_behind(
    repo: &gix::Repository,
    head_ref: Option<&gix::Reference<'_>>,
) -> Option<(usize, usize)> {
    let head_ref = head_ref?;
    let upstream_name = repo
        .branch_remote_tracking_ref_name(head_ref.name(), gix::remote::Direction::Fetch)?
        .ok()?;
    let upstream_id = repo
        .find_reference(upstream_name.as_ref())
        .ok()?
        .into_fully_peeled_id()
        .ok()?
        .detach();
    let head_id = repo.head_id().ok()?.detach();
    let ahead = repo
        .rev_walk([head_id])
        .with_hidden([upstream_id])
        .all()
        .ok()?
        .filter_map(Result::ok)
        .count();
    let behind = repo
        .rev_walk([upstream_id])
        .with_hidden([head_id])
        .all()
        .ok()?
        .filter_map(Result::ok)
        .count();
    Some((ahead, behind))
}

fn render_body(info: &StatusInfo, shape: Shape) -> Body {
    match shape {
        Shape::Badge => Body::Badge(BadgeData {
            status: if info.dirty {
                PayloadStatus::Warn
            } else {
                PayloadStatus::Ok
            },
            label: if info.dirty {
                format!("{} · dirty", short_branch(info))
            } else {
                format!("{} · clean", short_branch(info))
            },
        }),
        Shape::Text => text_body(format_line(info)),
        _ => Body::Entries(EntriesData {
            items: build_entries(info),
        }),
    }
}

fn build_entries(info: &StatusInfo) -> Vec<Entry> {
    let mut items = vec![
        entry(
            "branch",
            if info.detached {
                &info.head_short
            } else {
                &info.branch
            },
            None,
        ),
        entry(
            "state",
            if info.dirty { "dirty" } else { "clean" },
            Some(if info.dirty {
                PayloadStatus::Warn
            } else {
                PayloadStatus::Ok
            }),
        ),
    ];
    if !info.head_short.is_empty() && !info.detached {
        items.push(entry("head", &info.head_short, None));
    }
    if let (Some(a), Some(b)) = (info.ahead, info.behind) {
        items.push(entry("ahead", &a.to_string(), None));
        items.push(entry("behind", &b.to_string(), None));
    }
    items
}

fn format_line(info: &StatusInfo) -> String {
    let mut out = short_branch(info);
    if let (Some(a), Some(b)) = (info.ahead, info.behind)
        && (a > 0 || b > 0)
    {
        out.push_str(&format!(" ↑{a} ↓{b}"));
    }
    out.push_str(if info.dirty { " · dirty" } else { " · clean" });
    out
}

fn short_branch(info: &StatusInfo) -> String {
    if info.detached {
        format!("detached@{}", info.head_short)
    } else {
        info.branch.clone()
    }
}

fn entry(key: &str, value: &str, status: Option<PayloadStatus>) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::super::test_support::{commit, dirty_write, make_repo};
    use super::*;

    fn call(repo: &gix::Repository, shape: Shape) -> Body {
        let info = read_status(repo).unwrap();
        render_body(&info, shape)
    }

    fn info(
        detached: bool,
        dirty: bool,
        ahead: Option<usize>,
        behind: Option<usize>,
    ) -> StatusInfo {
        StatusInfo {
            branch: "feature/demo".into(),
            detached,
            head_short: "abc1234".into(),
            dirty,
            ahead,
            behind,
        }
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn clean_repo_entries() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        let body = call(&repo, Shape::Entries);
        let items = match body {
            Body::Entries(d) => d.items,
            _ => panic!(),
        };
        let map: std::collections::HashMap<_, _> = items
            .iter()
            .map(|e| (e.key.as_str(), e.value.as_deref().unwrap()))
            .collect();
        assert_eq!(map.get("branch"), Some(&"main"));
        assert_eq!(map.get("state"), Some(&"clean"));
    }

    #[test]
    fn dirty_repo_shows_warn_status() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        dirty_write(&repo, "README.md", "dirty\n");
        let body = call(&repo, Shape::Entries);
        let items = match body {
            Body::Entries(d) => d.items,
            _ => panic!(),
        };
        let state = items.iter().find(|e| e.key == "state").unwrap();
        assert_eq!(state.value.as_deref(), Some("dirty"));
        assert_eq!(state.status, Some(PayloadStatus::Warn));
    }

    #[test]
    fn badge_shape_reflects_dirty() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        dirty_write(&repo, "README.md", "dirty\n");
        let body = call(&repo, Shape::Badge);
        match body {
            Body::Badge(b) => {
                assert_eq!(b.status, PayloadStatus::Warn);
                assert!(b.label.contains("dirty"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_shape_summarises_branch_and_state() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        let body = call(&repo, Shape::Text);
        match body {
            Body::Text(d) => {
                assert!(d.value.contains("main"));
                assert!(d.value.contains("clean"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn catalog_surface_and_samples_match_status_contract() {
        let fetcher = GitStatus;
        let text_key = fetcher.cache_key(&FetchContext {
            shape: Some(Shape::Text),
            format: Some("plain".into()),
            ..FetchContext::default()
        });

        assert_eq!(fetcher.name(), "git_status");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("ahead/behind"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Entries);
        assert!(text_key.starts_with("git_status-"));
        assert_ne!(text_key, fetcher.cache_key(&FetchContext::default()));
        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Text),
            Some(Body::Text(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Badge),
            Some(Body::Badge(_))
        ));
        assert!(fetcher.sample_body(Shape::TextBlock).is_none());
    }

    #[test]
    fn detached_status_uses_short_head_and_clean_badge() {
        let detached = info(true, false, None, None);
        let zero_delta = info(false, false, Some(0), Some(0));
        let entries = build_entries(&detached);

        assert_eq!(entries[0].value.as_deref(), Some("abc1234"));
        assert!(entries.iter().all(|entry| entry.key != "head"));
        assert_eq!(format_line(&detached), "detached@abc1234 · clean");
        assert_eq!(format_line(&zero_delta), "feature/demo · clean");
        match render_body(&detached, Shape::Badge) {
            Body::Badge(badge) => {
                assert_eq!(badge.status, PayloadStatus::Ok);
                assert_eq!(badge.label, "detached@abc1234 · clean");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ahead_and_behind_are_rendered_in_entries_and_text() {
        let entries = build_entries(&info(false, true, Some(2), Some(1)));
        let map: std::collections::HashMap<_, _> = entries
            .iter()
            .map(|entry| (entry.key.as_str(), entry.value.as_deref().unwrap()))
            .collect();

        assert_eq!(map.get("branch"), Some(&"feature/demo"));
        assert_eq!(map.get("head"), Some(&"abc1234"));
        assert_eq!(map.get("ahead"), Some(&"2"));
        assert_eq!(map.get("behind"), Some(&"1"));
        assert_eq!(
            format_line(&info(false, true, Some(2), Some(1))),
            "feature/demo ↑2 ↓1 · dirty"
        );
        match render_body(&info(false, true, Some(2), Some(1)), Shape::Text) {
            Body::Text(text) => assert_eq!(text.value, "feature/demo ↑2 ↓1 · dirty"),
            _ => panic!(),
        }
    }

    #[test]
    fn read_status_reports_diverged_tracking_branch_and_detached_head() {
        let (tmp, repo) = make_repo();
        let dir = tmp.path();

        commit(&repo, "initial");
        run_git(dir, &["remote", "add", "origin", "."]);
        run_git(dir, &["config", "branch.main.remote", "origin"]);
        run_git(dir, &["config", "branch.main.merge", "refs/heads/main"]);
        run_git(dir, &["checkout", "-q", "-b", "remote-main"]);
        commit(&repo, "remote");
        run_git(
            dir,
            &[
                "update-ref",
                "refs/remotes/origin/main",
                "refs/heads/remote-main",
            ],
        );
        run_git(dir, &["checkout", "-q", "main"]);
        commit(&repo, "local");

        let repo = gix::discover(dir).unwrap();
        let status = read_status(&repo).unwrap();
        assert_eq!(status.branch, "main");
        assert_eq!(status.ahead, Some(1));
        assert_eq!(status.behind, Some(1));
        assert!(!status.detached);

        run_git(dir, &["checkout", "-q", "--detach", "HEAD~1"]);
        let repo = gix::discover(dir).unwrap();
        let detached = read_status(&repo).unwrap();
        assert_eq!(detached.branch, "HEAD");
        assert!(detached.detached);
        assert!(detached.ahead.is_none());
        assert!(detached.behind.is_none());
        assert!(!detached.head_short.is_empty());
    }
}
