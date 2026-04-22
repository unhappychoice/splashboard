use async_trait::async_trait;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::payload::{Body, HeatmapData, NumberSeriesData, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{fail, open_repo, payload, repo_cache_key, text_body};

const SHAPES: &[Shape] = &[Shape::Heatmap, Shape::NumberSeries, Shape::Text];
const WEEKS: usize = 52;
const DAYS_PER_WEEK: usize = 7;

/// Commit count per day over the last 52 weeks, anchored like GitHub: rightmost column contains
/// today's week, rows are weekdays (0 = Sunday .. 6 = Saturday). `Heatmap` is the default; the
/// `NumberSeries` alt flattens the grid into a 364-point array for sparkline rendering;
/// `Text` emits a one-line summary.
pub struct GitCommitsActivity;

#[async_trait]
impl Fetcher for GitCommitsActivity {
    fn name(&self) -> &str {
        "git_commits_activity"
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
            Shape::Heatmap => samples::heatmap_grid(7, 14),
            Shape::NumberSeries => samples::number_series(&[
                2, 5, 3, 8, 4, 9, 6, 11, 7, 3, 4, 6, 2, 5, 8, 10, 7, 4, 9, 6,
            ]),
            Shape::Text => samples::text("42 commits this month"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let repo = open_repo()?;
        let today = Local::now().date_naive();
        let grid = commits_grid(&repo, today)?;
        Ok(payload(render_body(
            grid,
            today,
            ctx.shape.unwrap_or(Shape::Heatmap),
        )))
    }
}

fn commits_grid(repo: &gix::Repository, today: NaiveDate) -> Result<Vec<Vec<u32>>, FetchError> {
    let mut grid = vec![vec![0u32; WEEKS]; DAYS_PER_WEEK];
    let Ok(head_id) = repo.head_id() else {
        return Ok(grid);
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
    for info in walker {
        let Ok(info) = info else { continue };
        let Ok(commit) = repo.find_commit(info.id) else {
            continue;
        };
        let Ok(time) = commit.time() else { continue };
        let Some(dt) = DateTime::<Utc>::from_timestamp(time.seconds, 0) else {
            continue;
        };
        let date = dt.with_timezone(&Local).date_naive();
        if let Some((row, col)) = cell_of(date, start, today) {
            grid[row][col] = grid[row][col].saturating_add(1);
        }
    }
    Ok(grid)
}

/// Sunday of the leftmost column so `today`'s week lands in column 51.
fn grid_start_date(today: NaiveDate) -> NaiveDate {
    let offset_from_sunday = today.weekday().num_days_from_sunday() as i64;
    today - Duration::days((WEEKS as i64 - 1) * 7 + offset_from_sunday)
}

fn cell_of(date: NaiveDate, start: NaiveDate, today: NaiveDate) -> Option<(usize, usize)> {
    if date < start || date > today {
        return None;
    }
    let days_from_start = (date - start).num_days();
    let col = (days_from_start / 7) as usize;
    let row = date.weekday().num_days_from_sunday() as usize;
    if col >= WEEKS || row >= DAYS_PER_WEEK {
        return None;
    }
    Some((row, col))
}

fn month_labels(start: NaiveDate) -> Vec<String> {
    let mut last_month = 0u32;
    (0..WEEKS)
        .map(|week| {
            let week_start = start + Duration::days((week as i64) * 7);
            for d in 0..DAYS_PER_WEEK {
                let day = week_start + Duration::days(d as i64);
                if day.day() == 1 && day.month() != last_month {
                    last_month = day.month();
                    return short_month(day.month());
                }
            }
            String::new()
        })
        .collect()
}

fn short_month(m: u32) -> String {
    [
        "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .get(m as usize)
    .copied()
    .unwrap_or("")
    .to_string()
}

fn render_body(grid: Vec<Vec<u32>>, today: NaiveDate, shape: Shape) -> Body {
    match shape {
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData {
            values: flatten_by_day(&grid),
        }),
        Shape::Text => {
            let total: u64 = grid.iter().flatten().map(|v| *v as u64).sum();
            if total == 0 {
                text_body("")
            } else {
                text_body(format!("{total} commits in the last {WEEKS} weeks"))
            }
        }
        _ => Body::Heatmap(HeatmapData {
            cells: grid,
            thresholds: None,
            row_labels: None,
            col_labels: Some(month_labels(grid_start_date(today))),
        }),
    }
}

/// Flattens a 7×52 grid into a 364-entry daily series in chronological order (oldest → newest).
fn flatten_by_day(grid: &[Vec<u32>]) -> Vec<u64> {
    (0..WEEKS)
        .flat_map(|col| (0..DAYS_PER_WEEK).map(move |row| grid[row][col] as u64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{commit, make_repo};
    use super::*;

    fn any_weekday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, 22).unwrap()
    }

    #[test]
    fn empty_repo_grid_is_all_zero() {
        let (_tmp, repo) = make_repo();
        let grid = commits_grid(&repo, any_weekday()).unwrap();
        assert_eq!(grid.len(), DAYS_PER_WEEK);
        assert!(grid.iter().all(|row| row.len() == WEEKS));
        assert!(grid.iter().flatten().all(|v| *v == 0));
    }

    #[test]
    fn today_commit_lands_in_rightmost_column() {
        let (_tmp, repo) = make_repo();
        commit(&repo, "today");
        let today = Local::now().date_naive();
        let grid = commits_grid(&repo, today).unwrap();
        let row = today.weekday().num_days_from_sunday() as usize;
        assert_eq!(grid[row][WEEKS - 1], 1, "today should be in last column");
    }

    #[test]
    fn grid_start_is_a_sunday() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap(); // Wednesday
        let start = grid_start_date(today);
        assert_eq!(start.weekday().num_days_from_sunday(), 0);
        assert_eq!((today - start).num_days(), (WEEKS as i64 - 1) * 7 + 3);
    }

    #[test]
    fn cell_of_past_anchor_in_first_column() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
        let start = grid_start_date(today);
        // The start date itself should be (row=0 Sun, col=0)
        assert_eq!(cell_of(start, start, today), Some((0, 0)));
    }

    #[test]
    fn cell_of_out_of_range_returns_none() {
        let today = NaiveDate::from_ymd_opt(2026, 4, 22).unwrap();
        let start = grid_start_date(today);
        assert!(cell_of(start - Duration::days(1), start, today).is_none());
        assert!(cell_of(today + Duration::days(1), start, today).is_none());
    }

    #[test]
    fn number_series_flatten_is_chronological() {
        let mut grid = vec![vec![0u32; WEEKS]; DAYS_PER_WEEK];
        grid[0][0] = 1; // Sunday, first week
        grid[1][0] = 2; // Monday, first week
        grid[0][1] = 3; // Sunday, second week
        let series = flatten_by_day(&grid);
        assert_eq!(series.len(), WEEKS * DAYS_PER_WEEK);
        assert_eq!(series[0], 1);
        assert_eq!(series[1], 2);
        assert_eq!(series[7], 3);
    }

    #[test]
    fn text_shape_summary() {
        let mut grid = vec![vec![0u32; WEEKS]; DAYS_PER_WEEK];
        grid[0][0] = 5;
        let body = render_body(grid, any_weekday(), Shape::Text);
        match body {
            Body::Text(d) => assert!(d.value.contains("5 commits")),
            _ => panic!(),
        }
    }

    #[test]
    fn heatmap_shape_carries_month_labels() {
        let grid = vec![vec![0u32; WEEKS]; DAYS_PER_WEEK];
        let body = render_body(grid, any_weekday(), Shape::Heatmap);
        match body {
            Body::Heatmap(d) => {
                assert_eq!(d.cells.len(), DAYS_PER_WEEK);
                assert_eq!(d.col_labels.as_ref().unwrap().len(), WEEKS);
            }
            _ => panic!(),
        }
    }
}
