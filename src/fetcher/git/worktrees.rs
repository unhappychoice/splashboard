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
    fn description(&self) -> &'static str {
        "Linked worktrees plus the main worktree, each with its checked-out branch and path. Useful when you juggle several feature branches in parallel checkouts."
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
    use std::path::Path;
    use std::process::Command;
    use std::time::Duration;

    use super::super::test_support::{commit, make_repo};
    use super::*;
    use tempfile::tempdir;

    fn context(shape: Shape) -> FetchContext {
        FetchContext {
            widget_id: "git-worktrees".into(),
            format: Some("plain".into()),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape: Some(shape),
            options: None,
            timezone: None,
            locale: None,
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

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        let fetcher = GitWorktrees;
        assert_eq!(fetcher.name(), "git_worktrees");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::Entries);
        assert!(fetcher.description().contains("Linked worktrees"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(_))
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(_))
        ));
        assert!(fetcher.sample_body(Shape::Text).is_none());

        let entries_key = fetcher.cache_key(&context(Shape::Entries));
        let text_block_key = fetcher.cache_key(&context(Shape::TextBlock));
        assert!(entries_key.starts_with("git_worktrees-"));
        assert_ne!(entries_key, text_block_key);
    }

    #[test]
    fn worktrees_include_linked_and_detached_checkouts() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "init");
        let root = repo.workdir().unwrap().to_path_buf();
        let linked_root = tempdir().unwrap();
        let feature = linked_root.path().join("feature");
        let detached = linked_root.path().join("detached");
        run_git(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "feat/demo",
                feature.to_str().unwrap(),
                "HEAD",
            ],
        );
        run_git(
            &root,
            &[
                "worktree",
                "add",
                "--detach",
                detached.to_str().unwrap(),
                "HEAD",
            ],
        );

        let repo = gix::discover(&root).unwrap();
        let list = worktrees(&repo).unwrap();
        assert_eq!(list.len(), 3);

        let feature_wt = list
            .iter()
            .find(|wt| wt.path == feature.display().to_string())
            .unwrap();
        assert_eq!(feature_wt.branch.as_deref(), Some("feat/demo"));

        let detached_wt = list
            .iter()
            .find(|wt| wt.path == detached.display().to_string())
            .unwrap();
        assert!(detached_wt.branch.is_none());
        assert_ne!(feature_wt.id, detached_wt.id);
    }

    #[test]
    fn detached_worktree_output_omits_branch_name() {
        let text_body = render_body(
            vec![WorktreeInfo {
                id: "detached".into(),
                branch: None,
                path: "/tmp/detached".into(),
            }],
            Shape::TextBlock,
        );
        let Body::TextBlock(text) = text_body else {
            panic!("expected text block");
        };
        assert_eq!(text.lines, vec!["detached @ /tmp/detached".to_string()]);

        let entries_body = render_body(
            vec![WorktreeInfo {
                id: "detached".into(),
                branch: None,
                path: "/tmp/detached".into(),
            }],
            Shape::Entries,
        );
        let Body::Entries(entries) = entries_body else {
            panic!("expected entries");
        };
        assert_eq!(entries.items[0].value.as_deref(), Some("/tmp/detached"));
    }
}
