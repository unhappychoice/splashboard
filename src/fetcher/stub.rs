use std::sync::Arc;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarChartData, BignumData, Body, GaugeData, ListData, ListItem, Payload, SparklineData,
    Status, TextData,
};

use super::{FetchContext, FetchError, Fetcher, Safety};

pub fn builtins() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(StaticText),
        Arc::new(ClockStub),
        Arc::new(DiskStub),
        Arc::new(GitCommitsStub),
        Arc::new(SystemStub),
        Arc::new(GithubPrsStub),
    ]
}

pub struct StaticText;

#[async_trait]
impl Fetcher for StaticText {
    fn name(&self) -> &str {
        "static"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let line = ctx.format.clone().unwrap_or_default();
        Ok(payload(Body::Text(TextData { lines: vec![line] })))
    }
}

pub struct ClockStub;

#[async_trait]
impl Fetcher for ClockStub {
    fn name(&self) -> &str {
        "clock"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
        Ok(payload(Body::Bignum(BignumData {
            text: "12:34".into(),
        })))
    }
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
        Ok(payload(Body::Gauge(GaugeData {
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
        Ok(payload(Body::Sparkline(SparklineData {
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
        Ok(payload(Body::List(ListData {
            items: vec![
                ListItem {
                    key: "os".into(),
                    value: Some("linux".into()),
                    status: ok,
                },
                ListItem {
                    key: "uptime".into(),
                    value: Some("3d 2h".into()),
                    status: ok,
                },
                ListItem {
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
        Ok(payload(Body::BarChart(BarChartData {
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
    use std::time::Duration;

    use super::*;

    fn ctx(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn static_text_uses_format_field() {
        let p = StaticText.fetch(&ctx(Some("Hello!"))).await.unwrap();
        match p.body {
            Body::Text(t) => assert_eq!(t.lines, vec!["Hello!".to_string()]),
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn clock_stub_emits_bignum() {
        let p = ClockStub.fetch(&ctx(None)).await.unwrap();
        assert!(matches!(p.body, Body::Bignum(_)));
    }

    #[test]
    fn builtins_cover_default_config_fetchers() {
        let fetchers = builtins();
        let names: Vec<&str> = fetchers.iter().map(|f| f.name()).collect();
        for expected in [
            "static",
            "clock",
            "disk",
            "git_commits",
            "system",
            "github_prs",
        ] {
            assert!(names.contains(&expected), "missing builtin: {expected}");
        }
    }

    #[test]
    fn safety_classification_marks_exec_and_network() {
        assert_eq!(GitCommitsStub.safety(), Safety::Exec);
        assert_eq!(GithubPrsStub.safety(), Safety::Network);
        assert_eq!(StaticText.safety(), Safety::Safe);
    }
}
