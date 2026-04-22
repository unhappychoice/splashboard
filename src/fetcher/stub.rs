//! Placeholder fetchers that still return canned data. Each of these is blocked on a dedicated
//! widget initiative and will be replaced one-by-one. Keeping them in a single module makes it
//! obvious at a glance which parts of the default dashboard aren't real yet.

use std::sync::Arc;

use async_trait::async_trait;

use crate::payload::{BadgeData, Body, HeatmapData, Payload, PointSeries, PointSeriesData, Status};
use crate::render::Shape;
use crate::samples;

use super::{FetchContext, FetchError, Fetcher, Safety};

pub fn stubs() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(TrendStub),
        Arc::new(ContributionsStub),
        Arc::new(CiStatusStub),
        Arc::new(DeployStatusStub),
        Arc::new(OncallStatusStub),
    ]
}

pub struct TrendStub;

#[async_trait]
impl Fetcher for TrendStub {
    fn name(&self) -> &str {
        "trend"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::PointSeries]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::PointSeries => Some(samples::sine_points()),
            _ => None,
        }
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(Body::PointSeries(PointSeriesData {
            series: vec![PointSeries {
                name: "series".into(),
                points: vec![
                    (0.0, 20.0),
                    (1.0, 22.5),
                    (2.0, 19.8),
                    (3.0, 24.1),
                    (4.0, 23.0),
                    (5.0, 25.6),
                    (6.0, 22.0),
                ],
            }],
        })))
    }
}

/// Demo-quality contributions grid — stands in for the future `github_contributions` and
/// `git_commits_activity` fetchers so the heatmap renderer has something to show out of the
/// box. 7 weekdays × 52 weeks, deterministic pseudo-random counts seeded so the picture looks
/// realistic (weekday peaks, weekends dim, occasional zero streaks).
pub struct ContributionsStub;

#[async_trait]
impl Fetcher for ContributionsStub {
    fn name(&self) -> &str {
        "contributions"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Heatmap]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Heatmap => Some(samples::heatmap_grid(7, 20)),
            _ => None,
        }
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(Body::Heatmap(HeatmapData {
            cells: fake_contributions(),
            thresholds: None,
            row_labels: None,
            col_labels: Some(month_labels_for_last_52_weeks()),
        })))
    }
}

fn fake_contributions() -> Vec<Vec<u32>> {
    // Deterministic LCG so the demo is stable across runs — visual regressions would be
    // confusing if the stub produced different pictures every splash.
    let mut state: u32 = 0x9E37_79B9;
    let mut next = |max: u32| -> u32 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 16) % max.max(1)
    };
    (0..7)
        .map(|day| {
            (0..52)
                .map(|_| {
                    let weekday_peak = if (1..=5).contains(&day) { 8 } else { 3 };
                    let roll = next(10);
                    if roll < 3 { 0 } else { next(weekday_peak) + 1 }
                })
                .collect()
        })
        .collect()
}

/// One string per week column, non-empty only on the week whose range contains the 1st of a
/// new month. The result slides with today's date so the demo always looks current.
fn month_labels_for_last_52_weeks() -> Vec<String> {
    use chrono::{Datelike, Duration, Local};
    let today = Local::now().date_naive();
    let start = today - Duration::days(51 * 7);
    let mut out: Vec<String> = (0..52).map(|_| String::new()).collect();
    let mut last_month = 0u32;
    for week in 0..52 {
        let week_start = start + Duration::days(week * 7);
        for d in 0..7 {
            let day = week_start + Duration::days(d);
            if day.day() == 1 && day.month() != last_month {
                out[week as usize] = short_month(day.month());
                last_month = day.month();
                break;
            }
        }
    }
    out
}

fn short_month(m: u32) -> String {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "",
    }
    .to_string()
}

/// Single-badge stubs — each pairs with the badge renderer one-to-one. Split into three
/// fetchers on purpose: a badge widget is "one indicator per fetcher". Mixing multiple
/// statuses into a single payload is the `combined_status_row` concern, handled at the
/// layout level, not in the data shape.
pub struct CiStatusStub;

#[async_trait]
impl Fetcher for CiStatusStub {
    fn name(&self) -> &str {
        "ci_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Badge]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Badge => Some(samples::badge(Status::Ok, "build passing")),
            _ => None,
        }
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(badge(Status::Ok, "build passing"))
    }
}

pub struct DeployStatusStub;

#[async_trait]
impl Fetcher for DeployStatusStub {
    fn name(&self) -> &str {
        "deploy_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Badge]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Badge => Some(samples::badge(Status::Warn, "deploy degraded")),
            _ => None,
        }
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(badge(Status::Warn, "deploy degraded"))
    }
}

pub struct OncallStatusStub;

#[async_trait]
impl Fetcher for OncallStatusStub {
    fn name(&self) -> &str {
        "oncall_status"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Badge]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Badge => Some(samples::badge(Status::Error, "oncall paging")),
            _ => None,
        }
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(badge(Status::Error, "oncall paging"))
    }
}

fn badge(status: Status, label: &str) -> Payload {
    payload(Body::Badge(BadgeData {
        status,
        label: label.into(),
    }))
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_cached_stubs_are_registered() {
        let fetchers = stubs();
        let names: Vec<&str> = fetchers.iter().map(|f| f.name()).collect();
        for expected in [
            "trend",
            "contributions",
            "ci_status",
            "deploy_status",
            "oncall_status",
        ] {
            assert!(names.contains(&expected), "missing stub: {expected}");
        }
    }

    #[test]
    fn contributions_stub_shape() {
        let cells = fake_contributions();
        assert_eq!(cells.len(), 7, "7 weekday rows");
        assert!(cells.iter().all(|r| r.len() == 52), "52 week columns");
        let total: u32 = cells.iter().flat_map(|r| r.iter().copied()).sum();
        assert!(total > 0, "deterministic fake data must not be all zero");
    }
}
