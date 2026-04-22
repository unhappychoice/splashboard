use std::collections::HashMap;
use std::convert::Infallible;
use std::ops::ControlFlow;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use gix::object::tree::diff::Change;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{Body, HeatmapData, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, lines_body, open_repo, payload, repo_cache_key};

const SHAPES: &[Shape] = &[Shape::Heatmap, Shape::Lines];
const WEEKS: usize = 52;
const TOP_FILES: usize = 7;
const MAX_COMMITS: usize = 500;

/// Per-file churn over the last 52 weeks. Rows are the top 7 most-touched files (truncated path),
/// columns are weeks (rightmost = this week), cell = commits in that week touching that file.
/// `Lines` emits a top-N summary `"src/main.rs (42), src/lib.rs (31), …"`.
pub struct GitBlameHeatmap;

#[async_trait]
impl Fetcher for GitBlameHeatmap {
    fn name(&self) -> &str {
        "git_blame_heatmap"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Heatmap
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Heatmap => samples::heatmap_grid(5, 10),
            Shape::Lines => samples::lines(&[
                "src/main.rs        18",
                "src/render/mod.rs  12",
                "src/fetcher/mod.rs  9",
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let today = Local::now().date_naive();
        let churn = churn(&repo, today)?;
        Ok(payload(render_body(
            churn,
            ctx.shape.unwrap_or(Shape::Heatmap),
        )))
    }
}

struct Churn {
    files: Vec<String>,
    totals: Vec<u32>,
    grid: Vec<Vec<u32>>,
}

fn churn(repo: &gix::Repository, today: NaiveDate) -> Result<Churn, FetchError> {
    let Ok(head_id) = repo.head_id() else {
        return Ok(Churn {
            files: Vec::new(),
            totals: Vec::new(),
            grid: Vec::new(),
        });
    };
    let start = grid_start_date(today);
    let cutoff = start.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
    let walker = repo
        .rev_walk([head_id.detach()])
        .sorting(Sorting::ByCommitTimeCutoff {
            seconds: cutoff,
            order: CommitTimeOrder::NewestFirst,
        })
        .all()
        .map_err(fail)?;

    let empty_tree = repo.empty_tree();
    let mut per_file_week: HashMap<String, Vec<u32>> = HashMap::new();
    let mut totals: HashMap<String, u32> = HashMap::new();

    for info in walker.take(MAX_COMMITS) {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let Ok(time) = commit.time() else { continue };
        let Some(dt) = DateTime::<Utc>::from_timestamp(time.seconds, 0) else {
            continue;
        };
        let date = dt.with_timezone(&Local).date_naive();
        let Some(col) = week_col(date, start) else {
            continue;
        };
        let Ok(tree) = commit.tree() else { continue };
        let parent_tree = commit
            .parent_ids()
            .next()
            .and_then(|id| repo.find_commit(id.detach()).ok())
            .and_then(|c| c.tree().ok())
            .unwrap_or_else(|| empty_tree.clone());

        let Ok(mut changes) = parent_tree.changes() else {
            continue;
        };
        let _ = changes.for_each_to_obtain_tree(&tree, |change| {
            if let Some(path) = changed_path(&change) {
                *totals.entry(path.clone()).or_insert(0) += 1;
                let weeks = per_file_week
                    .entry(path)
                    .or_insert_with(|| vec![0u32; WEEKS]);
                if let Some(slot) = weeks.get_mut(col) {
                    *slot = slot.saturating_add(1);
                }
            }
            Ok::<_, Infallible>(ControlFlow::Continue(()))
        });
    }

    let mut ranked: Vec<_> = totals.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(TOP_FILES);

    let files: Vec<String> = ranked.iter().map(|(f, _)| f.clone()).collect();
    let totals: Vec<u32> = ranked.iter().map(|(_, c)| *c).collect();
    let grid: Vec<Vec<u32>> = files
        .iter()
        .map(|f| per_file_week.remove(f).unwrap_or_else(|| vec![0u32; WEEKS]))
        .collect();
    Ok(Churn {
        files,
        totals,
        grid,
    })
}

fn changed_path(change: &Change<'_, '_, '_>) -> Option<String> {
    let (loc, mode) = match change {
        Change::Addition {
            location,
            entry_mode,
            ..
        }
        | Change::Deletion {
            location,
            entry_mode,
            ..
        }
        | Change::Modification {
            location,
            entry_mode,
            ..
        }
        | Change::Rewrite {
            location,
            entry_mode,
            ..
        } => (*location, entry_mode),
    };
    if loc.is_empty() || mode.is_tree() {
        None
    } else {
        Some(loc.to_string())
    }
}

fn grid_start_date(today: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    let offset_from_sunday = today.weekday().num_days_from_sunday() as i64;
    today - Duration::days((WEEKS as i64 - 1) * 7 + offset_from_sunday)
}

fn week_col(date: NaiveDate, start: NaiveDate) -> Option<usize> {
    let days = (date - start).num_days();
    if days < 0 {
        return None;
    }
    let col = (days / 7) as usize;
    if col >= WEEKS { None } else { Some(col) }
}

fn render_body(churn: Churn, shape: Shape) -> Body {
    match shape {
        Shape::Lines => {
            if churn.files.is_empty() {
                lines_body(Vec::new())
            } else {
                let line = churn
                    .files
                    .iter()
                    .zip(churn.totals.iter())
                    .map(|(f, c)| format!("{} ({c})", short_path(f)))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines_body(vec![line])
            }
        }
        _ => Body::Heatmap(HeatmapData {
            cells: churn.grid,
            thresholds: None,
            row_labels: Some(churn.files.into_iter().map(|f| short_path(&f)).collect()),
            col_labels: None,
        }),
    }
}

/// Truncate a long path to its basename so the heatmap row label stays readable.
fn short_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{commit_touching, make_repo};
    use super::*;

    #[test]
    fn empty_repo_returns_empty() {
        let (_tmp, repo) = make_repo();
        let c = churn(&repo, NaiveDate::from_ymd_opt(2026, 4, 22).unwrap()).unwrap();
        assert!(c.files.is_empty());
    }

    #[test]
    fn top_files_ranked_by_commit_count() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a.rs", "a1");
        commit_touching(&repo, "a.rs", "a2");
        commit_touching(&repo, "a.rs", "a3");
        commit_touching(&repo, "b.rs", "b1");
        let c = churn(&repo, Local::now().date_naive()).unwrap();
        assert_eq!(c.files[0], "a.rs");
        assert_eq!(c.totals[0], 3);
        assert_eq!(c.files[1], "b.rs");
        assert_eq!(c.totals[1], 1);
    }

    #[test]
    fn current_week_lands_in_rightmost_column() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a.rs", "a");
        let c = churn(&repo, Local::now().date_naive()).unwrap();
        assert_eq!(c.grid[0][WEEKS - 1], 1);
    }

    #[test]
    fn lines_shape_summarises_with_counts() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "dir/deep/name.rs", "x");
        let body = render_body(
            churn(&repo, Local::now().date_naive()).unwrap(),
            Shape::Lines,
        );
        match body {
            Body::Lines(d) => {
                assert_eq!(d.lines.len(), 1);
                assert!(
                    d.lines[0].starts_with("name.rs ("),
                    "unexpected line: {:?}",
                    d.lines[0]
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn heatmap_shape_carries_row_labels() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "x.rs", "x");
        let body = render_body(
            churn(&repo, Local::now().date_naive()).unwrap(),
            Shape::Heatmap,
        );
        match body {
            Body::Heatmap(d) => {
                assert_eq!(d.row_labels.as_ref().unwrap(), &vec!["x.rs".to_string()]);
                assert_eq!(d.cells[0].len(), WEEKS);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn short_path_returns_basename() {
        assert_eq!(short_path("a.rs"), "a.rs");
        assert_eq!(short_path("src/foo/bar.rs"), "bar.rs");
    }
}
