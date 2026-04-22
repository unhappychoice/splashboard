use async_trait::async_trait;

use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, Status as PayloadStatus};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, lines_body, open_repo, payload, repo_cache_key};

const SHAPES: &[Shape] = &[Shape::Entries, Shape::Lines, Shape::Badge];

/// Working tree snapshot: branch, dirty flag, ahead/behind vs upstream. Multi-shape so a user can
/// pick a key/value panel (`Entries`, default), a one-liner (`Lines`), or a clean/dirty traffic
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
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let info = read_status(&repo)?;
        Ok(payload(render_body(&info, ctx.shape.unwrap_or(Shape::Entries))))
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
        Shape::Lines => lines_body(vec![format_line(info)]),
        _ => Body::Entries(EntriesData {
            items: build_entries(info),
        }),
    }
}

fn build_entries(info: &StatusInfo) -> Vec<Entry> {
    let mut items = vec![
        entry(
            "branch",
            if info.detached { &info.head_short } else { &info.branch },
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
    use super::super::test_support::{commit, dirty_write, make_repo};
    use super::*;

    fn call(repo: &gix::Repository, shape: Shape) -> Body {
        let info = read_status(repo).unwrap();
        render_body(&info, shape)
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
        let map: std::collections::HashMap<_, _> =
            items.iter().map(|e| (e.key.as_str(), e.value.as_deref().unwrap())).collect();
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
    fn lines_shape_summarises_branch_and_state() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "initial");
        let body = call(&repo, Shape::Lines);
        match body {
            Body::Lines(d) => {
                assert_eq!(d.lines.len(), 1);
                assert!(d.lines[0].contains("main"));
                assert!(d.lines[0].contains("clean"));
            }
            _ => panic!(),
        }
    }
}
