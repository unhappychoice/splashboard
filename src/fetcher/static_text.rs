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
    fn description(&self) -> &'static str {
        "Renders a constant string supplied by the widget's `format` option, split into lines on `\\n` for `TextBlock`, collapsed to single-spaces for `Text`, and passed through verbatim for `MarkdownTextBlock`. Use it for greetings, project banners, or fixed welcome notes that don't need a dedicated fetcher."
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
                value: source.lines().collect::<Vec<_>>().join(" "),
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

    fn text_block(lines: &[&str]) -> Body {
        Body::TextBlock(TextBlockData {
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
        })
    }

    fn ctx(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "x".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
            ..Default::default()
        }
    }

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        assert_eq!(StaticText.name(), "basic_static");
        assert_eq!(StaticText.safety(), Safety::Safe);
        assert_eq!(StaticText.default_shape(), Shape::TextBlock);
        assert_eq!(
            StaticText.shapes(),
            &[Shape::Text, Shape::TextBlock, Shape::MarkdownTextBlock]
        );
        assert!(StaticText.description().contains("MarkdownTextBlock"));
        assert_eq!(
            StaticText.sample_body(Shape::Text),
            Some(Body::Text(TextData {
                value: "Hello, splashboard!".into(),
            }))
        );
        assert_eq!(
            StaticText.sample_body(Shape::TextBlock),
            Some(text_block(&["Hello, splashboard!"]))
        );
        assert_eq!(
            StaticText.sample_body(Shape::MarkdownTextBlock),
            Some(Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: "# Hello, splashboard!\n\nMarkdown body with **bold** and `code`.".into(),
            }))
        );
        assert_eq!(StaticText.sample_body(Shape::Badge), None);
    }

    #[tokio::test]
    async fn single_line_format() {
        let p = StaticText.fetch(&ctx(Some("Hello!"))).await.unwrap();
        assert_eq!(p.body, text_block(&["Hello!"]));
    }

    #[tokio::test]
    async fn newline_separates_into_lines() {
        let p = StaticText
            .fetch(&ctx(Some("line one\nline two\nline three")))
            .await
            .unwrap();
        assert_eq!(p.body, text_block(&["line one", "line two", "line three"]));
    }

    #[tokio::test]
    async fn missing_format_yields_empty_list() {
        let p = StaticText.fetch(&ctx(None)).await.unwrap();
        assert_eq!(p.body, text_block(&[]));
    }

    #[tokio::test]
    async fn empty_format_yields_empty_list() {
        let p = StaticText.fetch(&ctx(Some(""))).await.unwrap();
        assert_eq!(p.body, text_block(&[]));
    }

    #[tokio::test]
    async fn trailing_newline_preserves_blank_line() {
        let p = StaticText.fetch(&ctx(Some("a\n"))).await.unwrap();
        assert_eq!(p.body, text_block(&["a", ""]));
    }

    #[tokio::test]
    async fn text_shape_collapses_newlines_to_spaces() {
        let mut c = ctx(Some("line one\nline two"));
        c.shape = Some(Shape::Text);
        let p = StaticText.fetch(&c).await.unwrap();
        assert_eq!(
            p.body,
            Body::Text(TextData {
                value: "line one line two".into(),
            })
        );
    }

    #[tokio::test]
    async fn markdown_shape_preserves_verbatim_source() {
        let mut c = ctx(Some("# heading\n\nbody"));
        c.shape = Some(Shape::MarkdownTextBlock);
        let p = StaticText.fetch(&c).await.unwrap();
        assert_eq!(
            p.body,
            Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: "# heading\n\nbody".into(),
            })
        );
    }
}
