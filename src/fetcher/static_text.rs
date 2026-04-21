use async_trait::async_trait;

use crate::payload::{Body, LinesData, Payload};

use super::{FetchContext, FetchError, Fetcher, Safety};

/// Emits `format` verbatim, splitting on `\n` so users can ship multi-line fixed text blocks
/// ("welcome to this project", setup notes, etc.) without needing a dedicated fetcher. Empty
/// or missing format produces an empty line list — the widget renders nothing rather than a
/// spurious blank line.
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
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData { lines }),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "x".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
        }
    }

    fn text_lines(p: Payload) -> Vec<String> {
        match p.body {
            Body::Lines(t) => t.lines,
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn single_line_format() {
        let p = StaticText.fetch(&ctx(Some("Hello!"))).await.unwrap();
        assert_eq!(text_lines(p), vec!["Hello!".to_string()]);
    }

    #[tokio::test]
    async fn newline_separates_into_lines() {
        let p = StaticText
            .fetch(&ctx(Some("line one\nline two\nline three")))
            .await
            .unwrap();
        assert_eq!(
            text_lines(p),
            vec![
                "line one".to_string(),
                "line two".to_string(),
                "line three".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn missing_format_yields_empty_list() {
        let p = StaticText.fetch(&ctx(None)).await.unwrap();
        assert!(text_lines(p).is_empty());
    }

    #[tokio::test]
    async fn empty_format_yields_empty_list() {
        let p = StaticText.fetch(&ctx(Some(""))).await.unwrap();
        assert!(text_lines(p).is_empty());
    }

    #[tokio::test]
    async fn trailing_newline_preserves_blank_line() {
        // Users who don't want the trailing blank shouldn't trail a \n; we preserve split
        // semantics so the rendered output matches the format string byte-for-byte.
        let p = StaticText.fetch(&ctx(Some("a\n"))).await.unwrap();
        assert_eq!(text_lines(p), vec!["a".to_string(), "".to_string()]);
    }
}
