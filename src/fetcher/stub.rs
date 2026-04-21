//! Placeholder fetchers that still return canned data. Each of these is blocked on a dedicated
//! issue (#8 git, #9 system, #11 github) and will be replaced one-by-one. Keeping them in a
//! single module makes it obvious at a glance which parts of the default dashboard aren't real
//! yet.

use std::sync::Arc;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarsData, Body, CalendarData, EntriesData, Entry, NumberSeriesData, Payload, PointSeries,
    PointSeriesData, RatioData, Status,
};

use super::{FetchContext, FetchError, Fetcher, RealtimeFetcher, Safety};

pub fn stubs() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(DiskStub),
        Arc::new(GitCommitsStub),
        Arc::new(SystemStub),
        Arc::new(GithubPrsStub),
        Arc::new(TrendStub),
    ]
}

pub fn realtime_stubs() -> Vec<Arc<dyn RealtimeFetcher>> {
    vec![Arc::new(TodayStub)]
}

pub struct DiskStub;

#[async_trait]
impl Fetcher for DiskStub {
    fn name(&self) -> &str {
        "disk"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(Body::Ratio(RatioData {
            value: 0.45,
            label: Some("45% of 500 GB".into()),
        })))
    }
}

pub struct GitCommitsStub;

#[async_trait]
impl Fetcher for GitCommitsStub {
    fn name(&self) -> &str {
        "git_commits"
    }
    fn safety(&self) -> Safety {
        Safety::Exec
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(Body::NumberSeries(NumberSeriesData {
            values: vec![2, 5, 0, 3, 7, 4, 1, 6, 9, 2, 3, 5, 8, 4],
        })))
    }
}

pub struct SystemStub;

#[async_trait]
impl Fetcher for SystemStub {
    fn name(&self) -> &str {
        "system"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        let ok = Some(Status::Ok);
        Ok(payload(Body::Entries(EntriesData {
            items: vec![
                Entry {
                    key: "os".into(),
                    value: Some("linux".into()),
                    status: ok,
                },
                Entry {
                    key: "uptime".into(),
                    value: Some("3d 2h".into()),
                    status: ok,
                },
                Entry {
                    key: "load".into(),
                    value: Some("0.28".into()),
                    status: ok,
                },
            ],
        })))
    }
}

pub struct GithubPrsStub;

#[async_trait]
impl Fetcher for GithubPrsStub {
    fn name(&self) -> &str {
        "github_prs"
    }
    fn safety(&self) -> Safety {
        Safety::Network
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(Body::Bars(BarsData {
            bars: vec![
                Bar {
                    label: "splsh".into(),
                    value: 3,
                },
                Bar {
                    label: "gtype".into(),
                    value: 2,
                },
                Bar {
                    label: "other".into(),
                    value: 1,
                },
            ],
        })))
    }
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

pub struct TodayStub;

impl RealtimeFetcher for TodayStub {
    fn name(&self) -> &str {
        "today"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn compute(&self, _: &FetchContext) -> Payload {
        use chrono::Datelike;
        let now = chrono::Local::now().date_naive();
        payload(Body::Calendar(CalendarData {
            year: now.year(),
            month: now.month() as u8,
            day: Some(now.day() as u8),
            events: Vec::new(),
        }))
    }
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
    fn safety_classification_matches_feature_surface() {
        assert_eq!(DiskStub.safety(), Safety::Safe);
        assert_eq!(SystemStub.safety(), Safety::Safe);
        assert_eq!(GitCommitsStub.safety(), Safety::Exec);
        assert_eq!(GithubPrsStub.safety(), Safety::Network);
        assert_eq!(RealtimeFetcher::safety(&TodayStub), Safety::Safe);
    }

    #[test]
    fn all_cached_stubs_are_registered() {
        let fetchers = stubs();
        let names: Vec<&str> = fetchers.iter().map(|f| f.name()).collect();
        for expected in ["disk", "git_commits", "system", "github_prs", "trend"] {
            assert!(names.contains(&expected), "missing stub: {expected}");
        }
    }

    #[test]
    fn realtime_stubs_includes_today() {
        let names: Vec<String> = realtime_stubs()
            .iter()
            .map(|f| f.name().to_string())
            .collect();
        assert!(names.iter().any(|n| n == "today"));
    }
}
