use std::collections::HashMap;
use std::convert::Infallible;
use std::ops::ControlFlow;

use async_trait::async_trait;
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use gix::object::tree::diff::Change;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{Body, HeatmapData, Payload};
use crate::render::Shape;
use crate::samples;
use crate::time as t;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[Shape::Heatmap, Shape::Text];
const WEEKS: usize = 52;
const TOP_FILES: usize = 7;
const MAX_COMMITS: usize = 500;

/// Per-file churn over the last 52 weeks. Rows are the top 7 most-touched files (truncated path),
/// columns are weeks (rightmost = this week), cell = commits in that week touching that file.
/// `Text` emits a top-N summary `"src/main.rs (42), src/lib.rs (31), …"`.
pub struct GitBlameHeatmap;

#[async_trait]
impl Fetcher for GitBlameHeatmap {
    fn name(&self) -> &str {
        "git_blame_heatmap"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Per-file churn over the last 52 weeks: rows are the seven most-touched files, columns are weeks, cells are commit counts. Use it to spot hotspots; pair with `git_commits_activity` if you want overall cadence instead of per-file breakdown."
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
            Shape::Text => {
                samples::text("src/main.rs (18), src/render/mod.rs (12), src/fetcher/mod.rs (9)")
            }
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let now = t::now_in(ctx.timezone.as_deref());
        let churn = churn(&repo, now.date_naive(), *now.offset())?;
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

fn churn(repo: &gix::Repository, today: NaiveDate, tz: FixedOffset) -> Result<Churn, FetchError> {
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
        let date = dt.with_timezone(&tz).date_naive();
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
        Shape::Text => {
            if churn.files.is_empty() {
                text_body("")
            } else {
                let line = churn
                    .files
                    .iter()
                    .zip(churn.totals.iter())
                    .map(|(f, c)| format!("{} ({c})", short_path(f)))
                    .collect::<Vec<_>>()
                    .join(", ");
                text_body(line)
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
    use std::future::Future;

    use super::super::test_support::{commit_touching, make_repo};
    use super::*;

    fn local_now() -> chrono::DateTime<FixedOffset> {
        t::now_in(None)
    }

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
        let fetcher = GitBlameHeatmap;
        let default_key = fetcher.cache_key(&ctx(None));
        let text_key = fetcher.cache_key(&ctx(Some(Shape::Text)));

        assert_eq!(fetcher.name(), "git_blame_heatmap");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Per-file churn"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::Heatmap);
        assert!(default_key.starts_with("git_blame_heatmap-"));
        assert_ne!(default_key, text_key);
        assert_eq!(
            fetcher.sample_body(Shape::Heatmap),
            Some(samples::heatmap_grid(5, 10))
        );
        assert_eq!(
            fetcher.sample_body(Shape::Text),
            Some(samples::text(
                "src/main.rs (18), src/render/mod.rs (12), src/fetcher/mod.rs (9)",
            ))
        );
        assert!(fetcher.sample_body(Shape::Entries).is_none());
    }

    #[test]
    fn empty_repo_returns_empty() {
        let (_tmp, repo) = make_repo();
        let c = churn(
            &repo,
            NaiveDate::from_ymd_opt(2026, 4, 22).unwrap(),
            FixedOffset::east_opt(0).unwrap(),
        )
        .unwrap();
        assert!(c.files.is_empty());
    }

    #[test]
    fn top_files_ranked_by_commit_count() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a.rs", "a1");
        commit_touching(&repo, "a.rs", "a2");
        commit_touching(&repo, "a.rs", "a3");
        commit_touching(&repo, "b.rs", "b1");
        let now = local_now();
        let c = churn(&repo, now.date_naive(), *now.offset()).unwrap();
        assert_eq!(c.files[0], "a.rs");
        assert_eq!(c.totals[0], 3);
        assert_eq!(c.files[1], "b.rs");
        assert_eq!(c.totals[1], 1);
    }

    #[test]
    fn current_week_lands_in_rightmost_column() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a.rs", "a");
        let now = local_now();
        let c = churn(&repo, now.date_naive(), *now.offset()).unwrap();
        assert_eq!(c.grid[0][WEEKS - 1], 1);
    }

    #[test]
    fn text_shape_summarises_with_counts() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "dir/deep/name.rs", "x");
        let now = local_now();
        let body = render_body(
            churn(&repo, now.date_naive(), *now.offset()).unwrap(),
            Shape::Text,
        );
        assert!(matches!(&body, Body::Text(_)));
        if let Body::Text(d) = body {
            assert!(
                d.value.starts_with("name.rs ("),
                "unexpected text: {:?}",
                d.value
            );
        }
    }

    #[test]
    fn heatmap_shape_carries_row_labels() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "x.rs", "x");
        let now = local_now();
        let body = render_body(
            churn(&repo, now.date_naive(), *now.offset()).unwrap(),
            Shape::Heatmap,
        );
        assert!(matches!(&body, Body::Heatmap(_)));
        if let Body::Heatmap(d) = body {
            assert_eq!(d.row_labels.as_ref().unwrap(), &vec!["x.rs".to_string()]);
            assert_eq!(d.cells[0].len(), WEEKS);
        }
    }

    #[test]
    fn short_path_returns_basename() {
        assert_eq!(short_path("a.rs"), "a.rs");
        assert_eq!(short_path("src/foo/bar.rs"), "bar.rs");
    }

    #[test]
    fn week_col_rejects_dates_outside_the_heatmap_window() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
        let start = grid_start_date(today);

        assert_eq!(week_col(start, start), Some(0));
        assert_eq!(week_col(today, start), Some(WEEKS - 1));
        assert!(week_col(start - Duration::days(1), start).is_none());
        assert!(week_col(start + Duration::days((WEEKS * 7) as i64), start).is_none());
    }

    #[test]
    fn fetch_reads_workspace_repo_for_default_and_text_shapes() {
        let fetcher = GitBlameHeatmap;
        let repo = open_repo().unwrap();
        let now = local_now();

        let default_payload = run_async(fetcher.fetch(&ctx(None))).unwrap();
        let text_payload = run_async(fetcher.fetch(&ctx(Some(Shape::Text)))).unwrap();

        assert_eq!(
            default_payload,
            payload(render_body(
                churn(&repo, now.date_naive(), *now.offset()).unwrap(),
                Shape::Heatmap,
            ))
        );
        assert_eq!(
            text_payload,
            payload(render_body(
                churn(&repo, now.date_naive(), *now.offset()).unwrap(),
                Shape::Text,
            ))
        );
    }
}
