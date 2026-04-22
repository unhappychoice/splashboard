use async_trait::async_trait;

use crate::payload::{Body, EntriesData, Entry, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_block_body};

const SHAPES: &[Shape] = &[Shape::Entries, Shape::TextBlock];

/// Linked worktrees plus the main worktree. `Entries` maps worktree id → `"<branch> @ <path>"`.
/// `TextBlock` flattens to one worktree per line.
pub struct GitWorktrees;

#[async_trait]
impl Fetcher for GitWorktrees {
    fn name(&self) -> &str {
        "git_worktrees"
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
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("main", "/repo"),
                ("feat/foo", "/repo-feat"),
                ("hotfix", "/repo-hotfix"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "main           /repo",
                "feat/foo       /repo-feat",
                "hotfix         /repo-hotfix",
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let list = worktrees(&repo)?;
        Ok(payload(render_body(
            list,
            ctx.shape.unwrap_or(Shape::Entries),
        )))
    }
}

struct WorktreeInfo {
    id: String,
    branch: Option<String>,
    path: String,
}

fn worktrees(repo: &gix::Repository) -> Result<Vec<WorktreeInfo>, FetchError> {
    let mut out = Vec::new();
    if let Some(path) = repo.workdir() {
        out.push(WorktreeInfo {
            id: "(main)".into(),
            branch: head_branch(repo),
            path: path.display().to_string(),
        });
    }
    for proxy in repo.worktrees().map_err(fail)? {
        let id = proxy.id().to_string();
        let path = proxy
            .base()
            .ok()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let branch = proxy
            .into_repo_with_possibly_inaccessible_worktree()
            .ok()
            .as_ref()
            .and_then(head_branch);
        out.push(WorktreeInfo { id, branch, path });
    }
    Ok(out)
}

fn head_branch(repo: &gix::Repository) -> Option<String> {
    repo.head_ref()
        .ok()
        .flatten()
        .map(|r| r.name().shorten().to_string())
}

fn render_body(list: Vec<WorktreeInfo>, shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => text_block_body(list.into_iter().map(format_line).collect()),
        _ => Body::Entries(EntriesData {
            items: list
                .into_iter()
                .map(|wt| Entry {
                    key: wt.id,
                    value: Some(format_value(&wt.branch, &wt.path)),
                    status: None,
                })
                .collect(),
        }),
    }
}

fn format_line(wt: WorktreeInfo) -> String {
    match wt.branch {
        Some(b) => format!("{} ({}) @ {}", wt.id, b, wt.path),
        None => format!("{} @ {}", wt.id, wt.path),
    }
}

fn format_value(branch: &Option<String>, path: &str) -> String {
    match branch {
        Some(b) => format!("{b} @ {path}"),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{commit, make_repo};
    use super::*;

    #[test]
    fn returns_main_worktree_only_for_plain_repo() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "init");
        let list = worktrees(&repo).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "(main)");
        assert_eq!(list[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn entries_shape_key_is_id() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "init");
        let body = render_body(worktrees(&repo).unwrap(), Shape::Entries);
        match body {
            Body::Entries(d) => {
                assert_eq!(d.items[0].key, "(main)");
                let value = d.items[0].value.as_deref().unwrap();
                assert!(value.starts_with("main @ "));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_shape_formats_each_entry() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "init");
        let body = render_body(worktrees(&repo).unwrap(), Shape::TextBlock);
        match body {
            Body::TextBlock(d) => {
                assert_eq!(d.lines.len(), 1);
                assert!(d.lines[0].starts_with("(main) (main) @ "));
            }
            _ => panic!(),
        }
    }
}
