//! `hackernews_user_submissions` — recent stories submitted by an HN account. Walks the
//! `submitted` array on `/user/{login}.json`, fetches each item, and filters to story-shaped
//! types (`story` / `show_hn` / `ask_hn` / `job`). Comments are emitted by `hackernews_user_comments`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

use super::client::{API_BASE, HN_ITEM_URL, get};

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;
/// HN's `submitted` array can be long; cap how many items we try before giving up so a
/// stale-but-prolific account doesn't fan out hundreds of item requests just to find a
/// handful of stories.
const SCAN_LIMIT: usize = 80;

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string (HN login)",
        required: true,
        default: None,
        description: "Hacker News login whose submissions to list.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of stories to display.",
    },
];

pub struct HackernewsUserSubmissionsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[async_trait]
impl Fetcher for HackernewsUserSubmissionsFetcher {
    fn name(&self) -> &str {
        "hackernews_user_submissions"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Recent stories submitted by one Hacker News account (story / show / ask / job — comments are excluded). Use `hackernews_user_comments` for that user's recent comments instead."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "234pt 56c  Show HN: I built a thing",
                    Some("https://example.com/show-hn"),
                ),
                (
                    "187pt 41c  Why X over Y",
                    Some("https://news.ycombinator.com/item?id=2"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "234pt 56c  Show HN: I built a thing",
                "187pt 41c  Why X over Y",
            ]),
            Shape::Entries => samples::entries(&[
                ("Show HN: I built a thing", "234pt 56c"),
                ("Why X over Y", "187pt 41c"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let login = opts.user.ok_or_else(|| {
            FetchError::Failed("hackernews_user_submissions requires `user` option".into())
        })?;
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let stories = fetch_stories(&login, count).await?;
        Ok(payload(render_body(
            &stories,
            ctx.shape.unwrap_or(Shape::LinkedTextBlock),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct UserStub {
    #[serde(default)]
    submitted: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct Item {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default, rename = "type")]
    item_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    score: Option<u64>,
    #[serde(default)]
    descendants: Option<u64>,
    #[serde(default)]
    url: Option<String>,
}

async fn fetch_stories(login: &str, want: usize) -> Result<Vec<Item>, FetchError> {
    let user: UserStub = get(&format!("{API_BASE}/user/{login}.json")).await?;
    let candidates: Vec<u64> = user.submitted.into_iter().take(SCAN_LIMIT).collect();
    let handles: Vec<_> = candidates
        .into_iter()
        .map(|id| tokio::spawn(async move { fetch_item(id).await }))
        .collect();
    let mut stories = Vec::with_capacity(want);
    for h in handles {
        if stories.len() >= want {
            break;
        }
        if let Ok(Ok(it)) = h.await
            && is_story(&it)
        {
            stories.push(it);
        }
    }
    Ok(stories)
}

async fn fetch_item(id: u64) -> Result<Item, FetchError> {
    get(&format!("{API_BASE}/item/{id}.json")).await
}

fn is_story(it: &Item) -> bool {
    matches!(
        it.item_type.as_deref(),
        Some("story") | Some("job") | Some("show_hn") | Some("ask_hn")
    )
}

fn render_body(items: &[Item], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: items
                .iter()
                .map(|it| Entry {
                    key: title_or_placeholder(it),
                    value: Some(meta_label(it)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: items
                .iter()
                .map(|it| LinkedLine {
                    text: format!("{}  {}", meta_label(it), title_or_placeholder(it)),
                    url: link_for(it),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: items
                .iter()
                .map(|it| format!("{}  {}", meta_label(it), title_or_placeholder(it)))
                .collect(),
        }),
    }
}

fn title_or_placeholder(it: &Item) -> String {
    it.title.clone().unwrap_or_else(|| "(no title)".into())
}

fn meta_label(it: &Item) -> String {
    let score = it.score.unwrap_or(0);
    let comments = it.descendants.unwrap_or(0);
    format!("{score}pt {comments}c")
}

fn link_for(it: &Item) -> Option<String> {
    it.url
        .clone()
        .or_else(|| it.id.map(|id| format!("{HN_ITEM_URL}{id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::FetchContext;

    fn item(item_type: &str, title: Option<&str>) -> Item {
        Item {
            id: Some(1),
            item_type: Some(item_type.into()),
            title: title.map(String::from),
            score: Some(10),
            descendants: Some(2),
            url: None,
        }
    }

    fn ctx(shape: Shape, options: &str) -> FetchContext {
        FetchContext {
            shape: Some(shape),
            options: Some(toml::from_str(options).unwrap()),
            ..FetchContext::default()
        }
    }

    #[test]
    fn options_require_user() {
        let opts: Options = toml::from_str("user = \"pg\"\ncount = 5").unwrap();
        assert_eq!(opts.user.as_deref(), Some("pg"));
        assert_eq!(opts.count, Some(5));
    }

    #[test]
    fn is_story_accepts_story_job_show_ask() {
        assert!(is_story(&item("story", Some("x"))));
        assert!(is_story(&item("job", Some("x"))));
        assert!(is_story(&item("show_hn", Some("x"))));
        assert!(is_story(&item("ask_hn", Some("x"))));
    }

    #[test]
    fn is_story_rejects_comment_and_poll() {
        assert!(!is_story(&item("comment", None)));
        assert!(!is_story(&item("poll", Some("x"))));
    }

    #[test]
    fn linked_text_block_link_falls_back_to_hn_item_page() {
        let it = item("story", Some("hello"));
        assert_eq!(
            render_body(&[it], Shape::LinkedTextBlock),
            Body::LinkedTextBlock(LinkedTextBlockData {
                items: vec![LinkedLine {
                    text: "10pt 2c  hello".into(),
                    url: Some("https://news.ycombinator.com/item?id=1".into()),
                }],
            })
        );
    }

    #[test]
    fn linked_text_block_prefers_explicit_story_url() {
        let it = Item {
            id: Some(1),
            item_type: Some("story".into()),
            title: Some("hi".into()),
            score: Some(1),
            descendants: Some(0),
            url: Some("https://example.com/x".into()),
        };
        assert_eq!(
            render_body(&[it], Shape::LinkedTextBlock),
            Body::LinkedTextBlock(LinkedTextBlockData {
                items: vec![LinkedLine {
                    text: "1pt 0c  hi".into(),
                    url: Some("https://example.com/x".into()),
                }],
            })
        );
    }

    #[test]
    fn text_block_shape_includes_meta_and_title() {
        assert_eq!(
            render_body(&[item("story", Some("hello"))], Shape::TextBlock),
            Body::TextBlock(TextBlockData {
                lines: vec!["10pt 2c  hello".into()],
            })
        );
    }

    #[test]
    fn entries_shape_uses_title_as_key_and_meta_as_value() {
        assert_eq!(
            render_body(&[item("story", Some("hello"))], Shape::Entries),
            Body::Entries(EntriesData {
                items: vec![Entry {
                    key: "hello".into(),
                    value: Some("10pt 2c".into()),
                    status: None,
                }],
            })
        );
    }

    #[test]
    fn missing_title_falls_back_to_placeholder() {
        assert_eq!(
            render_body(&[item("story", None)], Shape::TextBlock),
            Body::TextBlock(TextBlockData {
                lines: vec!["10pt 2c  (no title)".into()],
            })
        );
    }

    #[test]
    fn empty_items_renders_empty_body() {
        assert_eq!(
            render_body(&[], Shape::LinkedTextBlock),
            Body::LinkedTextBlock(LinkedTextBlockData { items: vec![] })
        );
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("user = \"pg\"\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn item_deserializes_real_story_payload() {
        let raw = r#"{"id":42,"type":"story","title":"hi","score":99,"descendants":12,"url":"https://example.com"}"#;
        let it: Item = serde_json::from_str(raw).unwrap();
        assert_eq!(it.id, Some(42));
        assert_eq!(it.item_type.as_deref(), Some("story"));
        assert_eq!(it.title.as_deref(), Some("hi"));
        assert_eq!(it.score, Some(99));
        assert_eq!(it.descendants, Some(12));
        assert_eq!(it.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn user_stub_deserializes_partial_payload() {
        let raw = r#"{"id":"pg","submitted":[1,2,3]}"#;
        let stub: UserStub = serde_json::from_str(raw).unwrap();
        assert_eq!(stub.submitted, vec![1, 2, 3]);
    }

    #[test]
    fn fetcher_catalog_surface_matches_contract() {
        let fetcher = HackernewsUserSubmissionsFetcher;
        assert_eq!(fetcher.name(), "hackernews_user_submissions");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 2);
        assert_eq!(fetcher.option_schemas()[0].name, "user");
        assert_eq!(fetcher.option_schemas()[1].name, "count");
        assert!(fetcher.description().contains("comments are excluded"));
    }

    #[test]
    fn sample_body_supports_each_declared_shape() {
        let fetcher = HackernewsUserSubmissionsFetcher;

        assert_eq!(
            fetcher.sample_body(Shape::LinkedTextBlock),
            Some(Body::LinkedTextBlock(LinkedTextBlockData {
                items: vec![
                    LinkedLine {
                        text: "234pt 56c  Show HN: I built a thing".into(),
                        url: Some("https://example.com/show-hn".into()),
                    },
                    LinkedLine {
                        text: "187pt 41c  Why X over Y".into(),
                        url: Some("https://news.ycombinator.com/item?id=2".into()),
                    },
                ],
            }))
        );

        assert_eq!(
            fetcher.sample_body(Shape::TextBlock),
            Some(Body::TextBlock(TextBlockData {
                lines: vec![
                    "234pt 56c  Show HN: I built a thing".into(),
                    "187pt 41c  Why X over Y".into(),
                ],
            }))
        );

        assert_eq!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(EntriesData {
                items: vec![
                    Entry {
                        key: "Show HN: I built a thing".into(),
                        value: Some("234pt 56c".into()),
                        status: None,
                    },
                    Entry {
                        key: "Why X over Y".into(),
                        value: Some("187pt 41c".into()),
                        status: None,
                    },
                ],
            }))
        );

        assert!(fetcher.sample_body(Shape::Timeline).is_none());
    }

    #[test]
    fn cache_key_changes_with_shape_and_options() {
        let fetcher = HackernewsUserSubmissionsFetcher;
        let base = fetcher.cache_key(&ctx(Shape::TextBlock, "user = \"pg\"\ncount = 5"));
        let different_shape = fetcher.cache_key(&ctx(Shape::Entries, "user = \"pg\"\ncount = 5"));
        let different_options =
            fetcher.cache_key(&ctx(Shape::TextBlock, "user = \"pg\"\ncount = 6"));

        assert_ne!(base, different_shape);
        assert_ne!(base, different_options);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options() {
        let fetcher = HackernewsUserSubmissionsFetcher;
        let err = fetcher
            .fetch(&ctx(
                Shape::Entries,
                "user = \"pg\"\ncount = 5\nbogus = true",
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("unknown field `bogus`")
        ));
    }

    #[tokio::test]
    async fn fetch_requires_user_option() {
        let fetcher = HackernewsUserSubmissionsFetcher;
        let err = fetcher
            .fetch(&ctx(Shape::Entries, "count = 5"))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg)
                if msg == "hackernews_user_submissions requires `user` option"
        ));
    }

    #[tokio::test]
    async fn fetch_with_bad_login_surfaces_hn_failure() {
        let fetcher = HackernewsUserSubmissionsFetcher;
        for raw in ["user = \"bad login\"", "user = \"bad login\"\ncount = 999"] {
            let err = fetcher
                .fetch(&ctx(Shape::LinkedTextBlock, raw))
                .await
                .unwrap_err();
            assert!(matches!(
                err,
                FetchError::Failed(msg)
                    if msg.contains("hn request failed")
                        || msg.contains("hn json parse")
                        || msg.starts_with("hn ")
            ));
        }
    }

    #[test]
    fn meta_label_and_links_fall_back_when_fields_are_missing() {
        let it = Item {
            id: None,
            item_type: None,
            title: None,
            score: None,
            descendants: None,
            url: None,
        };
        assert_eq!(title_or_placeholder(&it), "(no title)");
        assert_eq!(meta_label(&it), "0pt 0c");
        assert_eq!(link_for(&it), None);
    }

    #[test]
    fn entries_shape_marks_rows_as_plain_status() {
        assert_eq!(
            render_body(&[item("story", Some("hello"))], Shape::Entries),
            Body::Entries(EntriesData {
                items: vec![Entry {
                    key: "hello".into(),
                    value: Some("10pt 2c".into()),
                    status: None,
                }],
            })
        );
    }

    #[test]
    fn user_stub_defaults_missing_submitted_to_empty() {
        let stub: UserStub = serde_json::from_str(r#"{}"#).unwrap();
        assert!(stub.submitted.is_empty());
    }
}
