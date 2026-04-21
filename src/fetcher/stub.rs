//! Placeholder fetchers that still return canned data. Each of these is blocked on a dedicated
//! issue (#8 git, #9 system, #11 github) and will be replaced one-by-one. Keeping them in a
//! single module makes it obvious at a glance which parts of the default dashboard aren't real
//! yet.

use std::sync::Arc;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, NumberSeriesData, Payload, RatioData, Status,
};

use super::{FetchContext, FetchError, Fetcher, Safety};

pub fn stubs() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(DiskStub),
        Arc::new(GitCommitsStub),
        Arc::new(SystemStub),
        Arc::new(GithubPrsStub),
    ]
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
    }

    #[test]
    fn all_stubs_are_registered() {
        let fetchers = stubs();
        let names: Vec<&str> = fetchers.iter().map(|f| f.name()).collect();
        for expected in ["disk", "git_commits", "system", "github_prs"] {
            assert!(names.contains(&expected), "missing stub: {expected}");
        }
    }
}
