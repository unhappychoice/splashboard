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
        Arc::new(ClockFetcher),
        Arc::new(DiskStub),
        Arc::new(GitCommitsStub),
        Arc::new(SystemStub),
        Arc::new(GithubPrsStub),
    ]
}

/// Emits `format` verbatim, splitting on `\n` so users can ship multi-line fixed text blocks
/// ("welcome to this project", setup notes, etc.) without needing a dedicated fetcher.
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
        let source = ctx.format.as_deref().unwrap_or("");
        let lines = if source.is_empty() {
            Vec::new()
        } else {
            source.split('\n').map(String::from).collect()
        };
        Ok(payload(Body::Text(TextData { lines })))
    }
}

/// Renders the current local time. `format` follows chrono's strftime conventions; default is
/// `%H:%M` (24h clock). Emits a Bignum payload so the default layout can lean on the big-text
/// renderer for visual weight.
pub struct ClockFetcher;

const CLOCK_DEFAULT_FORMAT: &str = "%H:%M";

#[async_trait]
impl Fetcher for ClockFetcher {
    fn name(&self) -> &str {
        "clock"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let fmt = ctx.format.as_deref().unwrap_or(CLOCK_DEFAULT_FORMAT);
        let text = chrono::Local::now().format(fmt).to_string();
        Ok(payload(Body::Bignum(BignumData { text })))
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
    async fn static_text_single_line() {
        let p = StaticText.fetch(&ctx(Some("Hello!"))).await.unwrap();
        match p.body {
            Body::Text(t) => assert_eq!(t.lines, vec!["Hello!".to_string()]),
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn static_text_splits_on_newline() {
        let p = StaticText
            .fetch(&ctx(Some("line one\nline two\nline three")))
            .await
            .unwrap();
        match p.body {
            Body::Text(t) => {
                assert_eq!(
                    t.lines,
                    vec![
                        "line one".to_string(),
                        "line two".to_string(),
                        "line three".to_string(),
                    ]
                );
            }
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn static_text_missing_format_is_empty() {
        let p = StaticText.fetch(&ctx(None)).await.unwrap();
        match p.body {
            Body::Text(t) => assert!(t.lines.is_empty()),
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn static_text_empty_format_is_empty() {
        let p = StaticText.fetch(&ctx(Some(""))).await.unwrap();
        match p.body {
            Body::Text(t) => assert!(t.lines.is_empty()),
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn static_text_trailing_newline_keeps_empty_line() {
        // Users who don't want the trailing blank shouldn't trail a \n; we preserve split
        // semantics so the rendered output matches the format string byte-for-byte.
        let p = StaticText.fetch(&ctx(Some("a\n"))).await.unwrap();
        match p.body {
            Body::Text(t) => assert_eq!(t.lines, vec!["a".to_string(), "".to_string()]),
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn clock_default_format_is_hh_mm() {
        let p = ClockFetcher.fetch(&ctx(None)).await.unwrap();
        match p.body {
            Body::Bignum(d) => {
                assert_eq!(d.text.len(), 5, "{:?} should be HH:MM", d.text);
                assert_eq!(d.text.chars().nth(2), Some(':'));
            }
            _ => panic!("expected bignum body"),
        }
    }

    #[tokio::test]
    async fn clock_honors_custom_format() {
        let p = ClockFetcher.fetch(&ctx(Some("%Y"))).await.unwrap();
        match p.body {
            Body::Bignum(d) => assert_eq!(d.text.len(), 4, "{:?} should be 4-digit year", d.text),
            _ => panic!("expected bignum body"),
        }
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
