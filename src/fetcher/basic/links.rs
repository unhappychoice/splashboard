//! `basic_links` — user-authored bookmark list emitted as the row-shaped variants. Pairs with a
//! per-directory `.splashboard/dashboard.toml` so a repo can ship pinned URLs (docs, CI,
//! dashboards) that surface automatically on `cd`.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::ImageLinkedList,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "links",
    type_hint: "list of {label, url?, subtitle?, thumbnail?}",
    required: false,
    default: Some("[]"),
    description: "Pinned links rendered as rows. `label` is required; `url` makes the row OSC8-clickable in `LinkedTextBlock` / `ImageLinkedList`; `thumbnail` (file path) lights up the `ImageLinkedList` variant.",
}];

pub struct BasicLinks;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub links: Vec<LinkConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkConfig {
    pub label: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
}

impl RealtimeFetcher for BasicLinks {
    fn name(&self) -> &str {
        "basic_links"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a user-authored list of pinned links from `[widget.options].links`. `LinkedTextBlock` (default) and `ImageLinkedList` wrap rows with a URL in OSC 8 escape sequences so terminals surface them as clickable; `Text` headlines the first label, `TextBlock` is plain labels, `MarkdownTextBlock` emits `- [label](url)` bullets, `Entries` is label → url rows. Right for per-directory bookmarks (repo docs, CI dashboard, deploy console)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                ("GitHub", Some("https://github.com/")),
                ("Docs", Some("https://example.com/docs")),
            ]),
            Shape::Text => samples::text("GitHub"),
            Shape::TextBlock => samples::text_block(&["GitHub", "Docs"]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- [GitHub](https://github.com/)\n- [Docs](https://example.com/docs)",
            ),
            Shape::Entries => samples::entries(&[
                ("GitHub", "https://github.com/"),
                ("Docs", "https://example.com/docs"),
            ]),
            Shape::ImageLinkedList => samples::image_linked_list(&[
                ("GitHub", Some("https://github.com/"), None, Some("source")),
                (
                    "Docs",
                    Some("https://example.com/docs"),
                    None,
                    Some("reference"),
                ),
            ]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        common::bare(body_for_shape(&opts.links, shape))
    }
}

fn body_for_shape(links: &[LinkConfig], shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: links.first().map(|l| l.label.clone()).unwrap_or_default(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: links.iter().map(|l| l.label.clone()).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: links
                .iter()
                .map(markdown_bullet)
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: links
                .iter()
                .map(|l| Entry {
                    key: l.label.clone(),
                    value: l.url.clone(),
                    status: None,
                })
                .collect(),
        }),
        Shape::ImageLinkedList => Body::ImageLinkedList(ImageLinkedListData {
            items: links
                .iter()
                .map(|l| ImageLinkedItem {
                    title: l.label.clone(),
                    url: l.url.clone(),
                    thumbnail_path: l.thumbnail.clone(),
                    subtitle: l.subtitle.clone(),
                })
                .collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: links
                .iter()
                .map(|l| LinkedLine {
                    text: l.label.clone(),
                    url: l.url.clone(),
                })
                .collect(),
        }),
    }
}

fn markdown_bullet(link: &LinkConfig) -> String {
    match &link.url {
        Some(url) => format!("- [{}]({})", link.label, url),
        None => format!("- {}", link.label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(options: toml::Value, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "x".into(),
            options: Some(options),
            shape,
            ..Default::default()
        }
    }

    fn opts(toml_src: &str) -> toml::Value {
        toml::from_str(toml_src).unwrap()
    }

    #[test]
    fn contract() {
        let f = BasicLinks;
        assert_eq!(f.name(), "basic_links");
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(f.shapes(), SHAPES);
    }

    /// Standard `basic_*` trio metadata cover: `description` mentions every shape variant
    /// `compute` can return so the catalog blurb stays in sync with the dispatch table, and
    /// `option_schemas` exposes the single `links` schema entry that the docs / TOML editor
    /// rely on. Pins the `&'static str` / `&[OptionSchema]` return paths that the bare contract
    /// test bypasses.
    #[test]
    fn metadata_methods_have_content() {
        let f = BasicLinks;
        let description = f.description();
        for needle in [
            "LinkedTextBlock",
            "ImageLinkedList",
            "Text",
            "TextBlock",
            "MarkdownTextBlock",
            "Entries",
        ] {
            assert!(
                description.contains(needle),
                "description missing {needle}: {description}",
            );
        }
        let schemas = f.option_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "links");
        assert!(!schemas[0].required);
    }

    /// `sample_body` returns `Some` for every declared shape and `None` for any shape outside
    /// `SHAPES` so the catalog page and the live preview can both rely on the slot being filled
    /// (or explicitly empty). Pins the wildcard `_ => return None` arm against future shapes
    /// landing in the enum before the match arm catches up.
    #[test]
    fn sample_body_covers_every_declared_shape() {
        let f = BasicLinks;
        for shape in SHAPES {
            assert!(
                f.sample_body(*shape).is_some(),
                "sample missing for {shape:?}",
            );
        }
        assert!(f.sample_body(Shape::Ratio).is_none());
        assert!(f.sample_body(Shape::Image).is_none());
    }

    #[test]
    fn linked_text_block_default_wraps_rows_with_optional_urls() {
        let p = BasicLinks.compute(&ctx(
            opts(
                r#"
                [[links]]
                label = "GitHub"
                url = "https://github.com/"
                [[links]]
                label = "Local Notes"
                "#,
            ),
            None,
        ));
        assert!(matches!(
            &p.body,
            Body::LinkedTextBlock(d)
                if d.items.len() == 2
                    && d.items[0].text == "GitHub"
                    && d.items[0].url.as_deref() == Some("https://github.com/")
                    && d.items[1].text == "Local Notes"
                    && d.items[1].url.is_none()
        ));
    }

    #[test]
    fn text_shape_headlines_first_link() {
        let p = BasicLinks.compute(&ctx(
            opts(
                r#"
                [[links]]
                label = "Docs"
                [[links]]
                label = "CI"
                "#,
            ),
            Some(Shape::Text),
        ));
        assert_eq!(
            p.body,
            Body::Text(TextData {
                value: "Docs".into()
            })
        );
    }

    #[test]
    fn markdown_bullets_use_link_syntax_only_when_url_set() {
        let p = BasicLinks.compute(&ctx(
            opts(
                r#"
                [[links]]
                label = "GitHub"
                url = "https://github.com/"
                [[links]]
                label = "TODO"
                "#,
            ),
            Some(Shape::MarkdownTextBlock),
        ));
        assert!(matches!(
            &p.body,
            Body::MarkdownTextBlock(d) if d.value == "- [GitHub](https://github.com/)\n- TODO"
        ));
    }

    #[test]
    fn text_block_shape_lists_labels_without_urls() {
        let p = BasicLinks.compute(&ctx(
            opts(
                r#"
                [[links]]
                label = "GitHub"
                url = "https://github.com/"
                [[links]]
                label = "Docs"
                "#,
            ),
            Some(Shape::TextBlock),
        ));
        assert!(matches!(
            &p.body,
            Body::TextBlock(d) if d.lines == ["GitHub".to_string(), "Docs".to_string()]
        ));
    }

    #[test]
    fn entries_shape_maps_label_to_optional_url() {
        let p = BasicLinks.compute(&ctx(
            opts(
                r#"
                [[links]]
                label = "GitHub"
                url = "https://github.com/"
                [[links]]
                label = "Docs"
                "#,
            ),
            Some(Shape::Entries),
        ));
        assert!(matches!(
            &p.body,
            Body::Entries(d)
                if d.items[0].key == "GitHub"
                    && d.items[0].value.as_deref() == Some("https://github.com/")
                    && d.items[1].key == "Docs"
                    && d.items[1].value.is_none()
        ));
    }

    #[test]
    fn missing_options_yields_empty_list() {
        let p = BasicLinks.compute(&FetchContext::default());
        assert!(matches!(&p.body, Body::LinkedTextBlock(d) if d.items.is_empty()));
    }

    #[test]
    fn unknown_option_key_renders_placeholder() {
        let p = BasicLinks.compute(&ctx(opts(r#"bogus = "x""#), None));
        assert!(matches!(
            &p.body,
            Body::TextBlock(d) if d.lines[0].contains("invalid options")
        ));
    }

    #[test]
    fn image_linked_list_carries_thumbnails_and_subtitles() {
        let p = BasicLinks.compute(&ctx(
            opts(
                r#"
                [[links]]
                label = "Logo"
                url = "https://example.com/"
                subtitle = "tagline"
                thumbnail = "/tmp/logo.png"
                "#,
            ),
            Some(Shape::ImageLinkedList),
        ));
        assert!(matches!(
            &p.body,
            Body::ImageLinkedList(d)
                if d.items[0].title == "Logo"
                    && d.items[0].thumbnail_path.as_deref() == Some("/tmp/logo.png")
                    && d.items[0].subtitle.as_deref() == Some("tagline")
        ));
    }
}
