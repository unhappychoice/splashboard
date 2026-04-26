use async_trait::async_trait;

use crate::payload::{Body, MarkdownTextBlockData, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

use super::{FetchContext, FetchError, Fetcher, Safety};

/// Emits `format` verbatim. Declares both text shapes: `Text` returns the entire format as one
/// string, `TextBlock` splits on `\n` so users can ship multi-line fixed text blocks ("welcome
/// to this project", setup notes, etc.) without needing a dedicated fetcher. Empty or missing
/// format produces empty output — the widget renders nothing rather than a spurious blank line.
pub struct StaticText;

#[async_trait]
impl Fetcher for StaticText {
    fn name(&self) -> &str {
        "basic_static"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text, Shape::TextBlock, Shape::MarkdownTextBlock]
    }
    fn default_shape(&self) -> Shape {
        Shape::TextBlock
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("Hello, splashboard!")),
            Shape::TextBlock => Some(samples::text_block(&["Hello, splashboard!"])),
            Shape::MarkdownTextBlock => Some(samples::markdown(
                "# Hello, splashboard!\n\nMarkdown body with **bold** and `code`.",
            )),
            _ => None,
        }
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let source = ctx.format.as_deref().unwrap_or("");
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: source.to_string(),
            }),
            Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: source.to_string(),
            }),
            _ => {
                let lines = if source.is_empty() {
                    Vec::new()
                } else {
                    source.split('\n').map(String::from).collect()
                };
                Body::TextBlock(TextBlockData { lines })
            }
        };
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body,
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
            ..Default::default()
        }
    }

    fn block_lines(p: Payload) -> Vec<String> {
        match p.body {
            Body::TextBlock(t) => t.lines,
            _ => panic!("expected text_block body"),
        }
    }

    #[tokio::test]
    async fn single_line_format() {
        let p = StaticText.fetch(&ctx(Some("Hello!"))).await.unwrap();
        assert_eq!(block_lines(p), vec!["Hello!".to_string()]);
    }

    #[tokio::test]
    async fn newline_separates_into_lines() {
        let p = StaticText
            .fetch(&ctx(Some("line one\nline two\nline three")))
            .await
            .unwrap();
        assert_eq!(
            block_lines(p),
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
        assert!(block_lines(p).is_empty());
    }

    #[tokio::test]
    async fn empty_format_yields_empty_list() {
        let p = StaticText.fetch(&ctx(Some(""))).await.unwrap();
        assert!(block_lines(p).is_empty());
    }

    #[tokio::test]
    async fn trailing_newline_preserves_blank_line() {
        let p = StaticText.fetch(&ctx(Some("a\n"))).await.unwrap();
        assert_eq!(block_lines(p), vec!["a".to_string(), "".to_string()]);
    }

    #[tokio::test]
    async fn text_shape_emits_whole_string() {
        let mut c = ctx(Some("line one\nline two"));
        c.shape = Some(Shape::Text);
        let p = StaticText.fetch(&c).await.unwrap();
        match p.body {
            Body::Text(t) => assert_eq!(t.value, "line one\nline two"),
            _ => panic!("expected text body"),
        }
    }
}
