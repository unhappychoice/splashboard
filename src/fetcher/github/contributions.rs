//! `github_contributions` — the GitHub contribution graph (草). GraphQL is the only API that
//! exposes the per-day counts; REST v3 has no equivalent. `Safe` (fixed query, fetches only
//! the authenticated user's own calendar).

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, HeatmapData, Payload};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::graphql;
use super::common::{cache_key, parse_options, payload, placeholder};

const SHAPES: &[Shape] = &[Shape::Heatmap];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "login",
    type_hint: "string",
    required: false,
    default: Some("viewer"),
    description: "GitHub login to fetch the contribution calendar for. Defaults to the authenticated user.",
}];

const QUERY_VIEWER: &str = "\
query { viewer { login contributionsCollection { contributionCalendar { weeks { contributionDays { date contributionCount } } } } } }";

const QUERY_USER: &str = "\
query($login: String!) { user(login: $login) { contributionsCollection { contributionCalendar { weeks { contributionDays { date contributionCount } } } } } }";

pub struct GithubContributions;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub login: Option<String>,
}

#[async_trait]
impl Fetcher for GithubContributions {
    fn name(&self) -> &str {
        "github_contributions"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let login = ctx
            .options
            .as_ref()
            .and_then(|v| v.get("login"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        cache_key(self.name(), ctx, login)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Heatmap => Some(samples::heatmap_grid(7, 52)),
            _ => None,
        }
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(placeholder(&msg)),
        };
        let calendar = match opts.login.as_deref() {
            None => fetch_viewer().await?,
            Some(login) => fetch_user(login).await?,
        };
        Ok(payload(Body::Heatmap(to_heatmap(&calendar))))
    }
}

#[derive(Debug, Deserialize)]
struct Calendar {
    weeks: Vec<Week>,
}

#[derive(Debug, Deserialize)]
struct Week {
    #[serde(rename = "contributionDays")]
    contribution_days: Vec<Day>,
}

#[derive(Debug, Deserialize)]
struct Day {
    date: String,
    #[serde(rename = "contributionCount")]
    contribution_count: u32,
}

async fn fetch_viewer() -> Result<Calendar, FetchError> {
    #[derive(Debug, Deserialize)]
    struct Viewer {
        viewer: UserNode,
    }
    #[derive(Debug, Deserialize)]
    struct UserNode {
        #[serde(rename = "contributionsCollection")]
        contributions_collection: Collection,
    }
    #[derive(Debug, Deserialize)]
    struct Collection {
        #[serde(rename = "contributionCalendar")]
        contribution_calendar: Calendar,
    }
    let data: Viewer = graphql(QUERY_VIEWER, serde_json::json!({})).await?;
    Ok(data.viewer.contributions_collection.contribution_calendar)
}

async fn fetch_user(login: &str) -> Result<Calendar, FetchError> {
    #[derive(Debug, Deserialize)]
    struct UserWrap {
        user: Option<UserNode>,
    }
    #[derive(Debug, Deserialize)]
    struct UserNode {
        #[serde(rename = "contributionsCollection")]
        contributions_collection: Collection,
    }
    #[derive(Debug, Deserialize)]
    struct Collection {
        #[serde(rename = "contributionCalendar")]
        contribution_calendar: Calendar,
    }
    let data: UserWrap =
        graphql(QUERY_USER, serde_json::json!({ "login": login })).await?;
    let user = data
        .user
        .ok_or_else(|| FetchError::Failed(format!("github: user {login:?} not found")))?;
    Ok(user.contributions_collection.contribution_calendar)
}

/// GraphQL returns weeks ordered oldest-to-newest with 7 days each (Sunday-first). We pivot to
/// `cells[weekday][week_index]` to match the heatmap renderer's row-major expectation.
fn to_heatmap(cal: &Calendar) -> HeatmapData {
    let weeks = cal.weeks.len();
    let mut cells: Vec<Vec<u32>> = (0..7).map(|_| vec![0; weeks]).collect();
    for (wi, week) in cal.weeks.iter().enumerate() {
        for day in &week.contribution_days {
            if let Some(weekday) = weekday_index(&day.date) {
                cells[weekday][wi] = day.contribution_count;
            }
        }
    }
    HeatmapData {
        cells,
        thresholds: None,
        row_labels: None,
        col_labels: Some(month_labels(cal)),
    }
}

fn weekday_index(raw: &str) -> Option<usize> {
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
    // ISO weekday: Mon=0..Sun=6. GitHub's weeks start Sunday, so shift to Sun=0..Sat=6.
    let wd = date.weekday().num_days_from_sunday() as usize;
    Some(wd)
}

/// One short month name per week column, set only on the week whose range crosses a month
/// boundary so the renderer can paint a top axis without overlapping labels.
fn month_labels(cal: &Calendar) -> Vec<String> {
    let mut out: Vec<String> = vec![String::new(); cal.weeks.len()];
    let mut last_month: u32 = 0;
    for (wi, week) in cal.weeks.iter().enumerate() {
        for day in &week.contribution_days {
            if let Some(date) = NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").ok()
                && date.day() == 1
                && date.month() != last_month
            {
                out[wi] = short_month(date.month());
                last_month = date.month();
                break;
            }
        }
    }
    out
}

fn short_month(m: u32) -> String {
    ["", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .get(m as usize)
        .copied()
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(date: &str, count: u32) -> Day {
        Day {
            date: date.into(),
            contribution_count: count,
        }
    }

    fn week(days: Vec<Day>) -> Week {
        Week {
            contribution_days: days,
        }
    }

    #[test]
    fn heatmap_has_seven_rows_and_week_count_columns() {
        let cal = Calendar {
            weeks: vec![week(vec![day("2026-04-19", 1), day("2026-04-20", 2)])],
        };
        let h = to_heatmap(&cal);
        assert_eq!(h.cells.len(), 7);
        assert!(h.cells.iter().all(|r| r.len() == 1));
    }

    #[test]
    fn sunday_lands_in_row_zero() {
        // 2026-04-19 is a Sunday.
        let cal = Calendar {
            weeks: vec![week(vec![day("2026-04-19", 9)])],
        };
        let h = to_heatmap(&cal);
        assert_eq!(h.cells[0][0], 9);
    }
}
