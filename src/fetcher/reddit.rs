//! `reddit_*` fetcher family (OAuth-free subset).
//!
//! Safety::Safe: requests target fixed Reddit hosts and options cannot redirect traffic.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{
    Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload, TextBlockData,
};
use crate::render::Shape;
use crate::samples;

const API_BASE: &str = "https://www.reddit.com";
const USER_AGENT: &str = concat!(
    "splashboard/",
    env!("CARGO_PKG_VERSION"),
    " (by /u/unhappychoice)"
);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_SUBREDDIT: &str = "programming";
const DEFAULT_USER: &str = "spez";
const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;
const DEFAULT_COMMENT_CHARS: usize = 110;
const MIN_COMMENT_CHARS: usize = 32;
const MAX_COMMENT_CHARS: usize = 240;

const SHAPES: &[Shape] = &[Shape::LinkedTextBlock, Shape::TextBlock, Shape::Entries];

const SUBREDDIT_POSTS_OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "subreddit",
        type_hint: "string",
        required: false,
        default: Some("\"programming\""),
        description: "Subreddit name without `/r/` prefix.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of posts to display.",
    },
    OptionSchema {
        name: "type",
        type_hint: "\"top\" | \"hot\" | \"new\" | \"rising\"",
        required: false,
        default: Some("\"top\""),
        description: "Subreddit listing type.",
    },
    OptionSchema {
        name: "period",
        type_hint: "\"hour\" | \"day\" | \"week\" | \"month\" | \"year\" | \"all\"",
        required: false,
        default: Some("\"day\""),
        description: "Ranking window used when `type = \"top\"`.",
    },
];

const USER_SUBMITTED_OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: false,
        default: Some("\"spez\""),
        description: "Reddit username (without `/u/` prefix).",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of submissions to display.",
    },
];

const USER_COMMENTS_OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "user",
        type_hint: "string",
        required: false,
        default: Some("\"spez\""),
        description: "Reddit username (without `/u/` prefix).",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of comments to display.",
    },
    OptionSchema {
        name: "max_chars",
        type_hint: "integer (32..=240)",
        required: false,
        default: Some("110"),
        description: "Max preview length per comment line.",
    },
];

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(RedditSubredditPostsFetcher),
        Arc::new(RedditSubredditTopFetcher),
        Arc::new(RedditUserSubmittedFetcher),
        Arc::new(RedditUserCommentsFetcher),
    ]
}

pub struct RedditSubredditPostsFetcher;
pub struct RedditSubredditTopFetcher;
pub struct RedditUserSubmittedFetcher;
pub struct RedditUserCommentsFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubredditPostsOptions {
    #[serde(default)]
    subreddit: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default, rename = "type")]
    r#type: Option<SubredditListingType>,
    #[serde(default)]
    period: Option<Period>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserSubmittedOptions {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    count: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserCommentsOptions {
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    count: Option<u32>,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SubredditListingType {
    #[default]
    Top,
    Hot,
    New,
    Rising,
}

impl SubredditListingType {
    fn path(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Hot => "hot",
            Self::New => "new",
            Self::Rising => "rising",
        }
    }

    fn needs_period(self) -> bool {
        matches!(self, Self::Top)
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Period {
    Hour,
    #[default]
    Day,
    Week,
    Month,
    Year,
    All,
}

impl Period {
    fn as_query(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
            Self::All => "all",
        }
    }
}

#[async_trait]
impl Fetcher for RedditSubredditPostsFetcher {
    fn name(&self) -> &str {
        "reddit_subreddit_posts"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        SUBREDDIT_POSTS_OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key_for(self.name(), ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_post_body(shape)
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        fetch_subreddit_posts_payload(ctx).await
    }
}

#[async_trait]
impl Fetcher for RedditSubredditTopFetcher {
    fn name(&self) -> &str {
        "reddit_subreddit_top"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        SUBREDDIT_POSTS_OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key_for(self.name(), ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_post_body(shape)
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        fetch_subreddit_posts_payload(ctx).await
    }
}

#[async_trait]
impl Fetcher for RedditUserSubmittedFetcher {
    fn name(&self) -> &str {
        "reddit_user_submitted"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        USER_SUBMITTED_OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key_for(self.name(), ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_post_body(shape)
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: UserSubmittedOptions =
            parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = normalized_count(opts.count);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let user = normalize_user(opts.user.as_deref().unwrap_or(DEFAULT_USER))?;
        match fetch_user_submitted(&user, count).await {
            Ok(posts) => Ok(payload(render_posts(&posts, shape))),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

#[async_trait]
impl Fetcher for RedditUserCommentsFetcher {
    fn name(&self) -> &str {
        "reddit_user_comments"
    }

    fn safety(&self) -> Safety {
        Safety::Safe
    }

    fn shapes(&self) -> &[Shape] {
        SHAPES
    }

    fn option_schemas(&self) -> &[OptionSchema] {
        USER_COMMENTS_OPTION_SCHEMAS
    }

    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key_for(self.name(), ctx)
    }

    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::LinkedTextBlock => samples::linked_text_block(&[
                (
                    "r/rust · 42↑  This migration guide made the update painless...",
                    Some("https://www.reddit.com/r/rust/comments/abc123/topic/def456/"),
                ),
                (
                    "r/programming · 18↑  +1 for clear release notes and rollback steps.",
                    Some("https://www.reddit.com/r/programming/comments/ghi789/topic/jkl012/"),
                ),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "r/rust · 42↑  This migration guide made the update painless...",
                "r/programming · 18↑  +1 for clear release notes and rollback steps.",
            ]),
            Shape::Entries => samples::entries(&[
                (
                    "This migration guide made the update painless...",
                    "r/rust · 42↑",
                ),
                (
                    "+1 for clear release notes and rollback steps.",
                    "r/programming · 18↑",
                ),
            ]),
            _ => return None,
        })
    }

    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: UserCommentsOptions =
            parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let count = normalized_count(opts.count);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let max_chars = opts
            .max_chars
            .unwrap_or(DEFAULT_COMMENT_CHARS)
            .clamp(MIN_COMMENT_CHARS, MAX_COMMENT_CHARS);
        let user = normalize_user(opts.user.as_deref().unwrap_or(DEFAULT_USER))?;
        match fetch_user_comments(&user, count).await {
            Ok(comments) => Ok(payload(render_comments(&comments, max_chars, shape))),
            Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
        }
    }
}

fn cache_key_for(name: &str, ctx: &FetchContext) -> String {
    let extra = ctx
        .options
        .as_ref()
        .and_then(|value| toml::to_string(value).ok())
        .unwrap_or_default();
    cache_key(name, ctx, &extra)
}

fn normalized_count(value: Option<u32>) -> usize {
    value.unwrap_or(DEFAULT_COUNT).clamp(MIN_COUNT, MAX_COUNT) as usize
}

async fn fetch_subreddit_posts_payload(ctx: &FetchContext) -> Result<Payload, FetchError> {
    let opts: SubredditPostsOptions =
        parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
    let count = normalized_count(opts.count);
    let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
    let listing_type = opts.r#type.unwrap_or_default();
    let period = opts.period.unwrap_or_default();
    let subreddit = normalize_subreddit(opts.subreddit.as_deref().unwrap_or(DEFAULT_SUBREDDIT))?;
    match fetch_subreddit_posts(&subreddit, count, listing_type, period).await {
        Ok(posts) => Ok(payload(render_posts(&posts, shape))),
        Err(err) => Ok(payload(network_unavailable_body(shape, &format!("{err}")))),
    }
}

fn network_unavailable_body(shape: Shape, err: &str) -> Body {
    let line = format!("⚠ {err}");
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![Entry {
                key: "reddit unavailable".into(),
                value: Some(err.into()),
                status: None,
            }],
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: line,
                url: None,
            }],
        }),
        _ => Body::TextBlock(TextBlockData { lines: vec![line] }),
    }
}

async fn fetch_subreddit_posts(
    subreddit: &str,
    count: usize,
    listing_type: SubredditListingType,
    period: Period,
) -> Result<Vec<Post>, FetchError> {
    let mut path = format!(
        "/r/{subreddit}/{}.json?limit={count}&raw_json=1",
        listing_type.path()
    );
    if listing_type.needs_period() {
        path.push_str("&t=");
        path.push_str(period.as_query());
    }
    fetch_listing_with_bases(&path).await
}

async fn fetch_user_submitted(user: &str, count: usize) -> Result<Vec<Post>, FetchError> {
    let path = format!("/user/{user}/submitted.json?limit={count}&raw_json=1");
    fetch_listing_with_bases(&path).await
}

async fn fetch_user_comments(user: &str, count: usize) -> Result<Vec<Comment>, FetchError> {
    let path = format!("/user/{user}/comments.json?limit={count}&raw_json=1");
    fetch_listing_with_bases(&path).await
}

async fn fetch_listing_with_bases<T: for<'de> Deserialize<'de>>(
    path_and_query: &str,
) -> Result<Vec<T>, FetchError> {
    let urls = vec![format!("{API_BASE}{path_and_query}")];
    let response: Listing<T> = get_json_with_fallback(&urls).await?;
    Ok(response
        .data
        .children
        .into_iter()
        .map(|child| child.data)
        .collect())
}

fn render_posts(posts: &[Post], shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: posts
                .iter()
                .map(|post| Entry {
                    key: post_title(post),
                    value: Some(post_meta(post)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: posts
                .iter()
                .map(|post| LinkedLine {
                    text: format!("{}  {}", post_meta(post), post_title(post)),
                    url: post_link(post),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: posts
                .iter()
                .map(|post| format!("{}  {}", post_meta(post), post_title(post)))
                .collect(),
        }),
    }
}

fn render_comments(comments: &[Comment], max_chars: usize, shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: comments
                .iter()
                .map(|comment| Entry {
                    key: truncate(
                        &comment.body.clone().unwrap_or_else(|| "(no text)".into()),
                        max_chars,
                    ),
                    value: Some(comment_meta(comment)),
                    status: None,
                })
                .collect(),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: comments
                .iter()
                .map(|comment| LinkedLine {
                    text: format!(
                        "{}  {}",
                        comment_meta(comment),
                        truncate(
                            &comment.body.clone().unwrap_or_else(|| "(no text)".into()),
                            max_chars
                        )
                    ),
                    url: comment
                        .permalink
                        .as_ref()
                        .map(|path| format!("https://www.reddit.com{path}")),
                })
                .collect(),
        }),
        _ => Body::TextBlock(TextBlockData {
            lines: comments
                .iter()
                .map(|comment| {
                    format!(
                        "{}  {}",
                        comment_meta(comment),
                        truncate(
                            &comment.body.clone().unwrap_or_else(|| "(no text)".into()),
                            max_chars
                        )
                    )
                })
                .collect(),
        }),
    }
}

fn sample_post_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => samples::linked_text_block(&[
            (
                "r/programming · 1520↑ 218c  Show: terminal dashboard with local trust model",
                Some("https://www.reddit.com/r/programming/comments/abc123/demo/"),
            ),
            (
                "r/rust · 934↑ 104c  Why async Rust feels different from Go",
                Some("https://example.com/async-rust-vs-go"),
            ),
        ]),
        Shape::TextBlock => samples::text_block(&[
            "r/programming · 1520↑ 218c  Show: terminal dashboard with local trust model",
            "r/rust · 934↑ 104c  Why async Rust feels different from Go",
        ]),
        Shape::Entries => samples::entries(&[
            (
                "Show: terminal dashboard with local trust model",
                "r/programming · 1520↑ 218c",
            ),
            (
                "Why async Rust feels different from Go",
                "r/rust · 934↑ 104c",
            ),
        ]),
        _ => return None,
    })
}

fn post_meta(post: &Post) -> String {
    let subreddit = post
        .subreddit
        .as_deref()
        .map(|name| format!("r/{name} · "))
        .unwrap_or_default();
    let score = post.score.unwrap_or(0);
    let comments = post.num_comments.unwrap_or(0);
    format!("{subreddit}{score}↑ {comments}c")
}

fn post_title(post: &Post) -> String {
    post.title.clone().unwrap_or_else(|| "(no title)".into())
}

fn post_link(post: &Post) -> Option<String> {
    post.url
        .clone()
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        .or_else(|| {
            post.permalink
                .as_ref()
                .map(|path| format!("https://www.reddit.com{path}"))
        })
}

fn comment_meta(comment: &Comment) -> String {
    let subreddit = comment
        .subreddit
        .as_deref()
        .map(|name| format!("r/{name} · "))
        .unwrap_or_default();
    let score = comment.score.unwrap_or(0);
    format!("{subreddit}{score}↑")
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.into();
    }
    let keep = max_chars.saturating_sub(1);
    let clipped: String = value.chars().take(keep).collect();
    format!("{clipped}…")
}

fn normalize_subreddit(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://www.reddit.com/r/")
        .trim_start_matches("/r/")
        .trim_matches('/');
    if trimmed.is_empty() {
        return Err(FetchError::Failed("subreddit must not be empty".into()));
    }
    if trimmed.len() > 21 {
        return Err(FetchError::Failed("subreddit is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(FetchError::Failed(
            "subreddit must contain only [A-Za-z0-9_]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn normalize_user(raw: &str) -> Result<String, FetchError> {
    let trimmed = raw
        .trim()
        .trim_start_matches("https://www.reddit.com/user/")
        .trim_start_matches("/u/")
        .trim_start_matches("u/")
        .trim_matches('/');
    if trimmed.is_empty() {
        return Err(FetchError::Failed("user must not be empty".into()));
    }
    if trimmed.len() > 20 {
        return Err(FetchError::Failed("user is too long".into()));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(FetchError::Failed(
            "user must contain only [A-Za-z0-9_-]".into(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

async fn get_json_with_fallback<T: serde::de::DeserializeOwned>(
    urls: &[String],
) -> Result<T, FetchError> {
    let mut last_error = "reddit request failed".to_string();
    for url in urls {
        for profile in [HeaderProfile::Minimal, HeaderProfile::Browserish] {
            match get_with_profile(url, profile).await {
                Ok(text) => {
                    return serde_json::from_str::<T>(&text)
                        .map_err(|e| FetchError::Failed(format!("reddit json parse: {e}")));
                }
                Err(FetchError::Failed(msg)) => {
                    last_error = msg;
                }
                Err(other) => return Err(other),
            }
        }
    }
    Err(FetchError::Failed(last_error))
}

#[derive(Clone, Copy)]
enum HeaderProfile {
    Minimal,
    Browserish,
}

async fn get_with_profile(url: &str, profile: HeaderProfile) -> Result<String, FetchError> {
    let mut req = http().get(url);
    if matches!(profile, HeaderProfile::Browserish) {
        req = req
            .header("accept", "application/json, text/plain, */*")
            .header("accept-language", "en-US,en;q=0.9")
            .header("cache-control", "no-cache")
            .header("pragma", "no-cache")
            .header("referer", "https://www.reddit.com/")
            .header("dnt", "1")
            .header("accept-encoding", "identity");
    }
    let res = req
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("reddit request failed: {e}")))?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(FetchError::Failed(format!(
            "reddit {status} @ {url} [{}]",
            match profile {
                HeaderProfile::Minimal => "minimal",
                HeaderProfile::Browserish => "browserish",
            }
        )));
    }
    if body.is_empty() {
        return Err(FetchError::Failed(format!("reddit empty body @ {url}")));
    }
    Ok(body)
}

#[derive(Debug, Deserialize)]
struct Listing<T> {
    data: ListingData<T>,
}

#[derive(Debug, Deserialize)]
struct ListingData<T> {
    children: Vec<Child<T>>,
}

#[derive(Debug, Deserialize)]
struct Child<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct Post {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    score: Option<u64>,
    #[serde(default)]
    num_comments: Option<u64>,
    #[serde(default)]
    subreddit: Option<String>,
    #[serde(default)]
    permalink: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Comment {
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    score: Option<i64>,
    #[serde(default)]
    subreddit: Option<String>,
    #[serde(default)]
    permalink: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subreddit_options_parse_type_and_period() {
        let raw: toml::Value =
            toml::from_str("subreddit = \"rust\"\ncount = 7\ntype = \"hot\"\nperiod = \"week\"")
                .unwrap();
        let opts: SubredditPostsOptions = raw.try_into().unwrap();
        assert_eq!(opts.subreddit.as_deref(), Some("rust"));
        assert_eq!(opts.count, Some(7));
        assert!(matches!(opts.r#type, Some(SubredditListingType::Hot)));
        assert!(matches!(opts.period, Some(Period::Week)));
    }

    #[test]
    fn user_comments_options_parse_max_chars() {
        let raw: toml::Value =
            toml::from_str("user = \"spez\"\ncount = 5\nmax_chars = 80").unwrap();
        let opts: UserCommentsOptions = raw.try_into().unwrap();
        assert_eq!(opts.user.as_deref(), Some("spez"));
        assert_eq!(opts.count, Some(5));
        assert_eq!(opts.max_chars, Some(80));
    }

    #[test]
    fn normalize_subreddit_accepts_r_prefix_and_lowercases() {
        assert_eq!(normalize_subreddit("/r/Rust").unwrap(), "rust");
    }

    #[test]
    fn normalize_user_accepts_u_prefix_and_lowercases() {
        assert_eq!(normalize_user("/u/Spez").unwrap(), "spez");
    }

    #[test]
    fn normalize_user_rejects_invalid_chars() {
        let err = normalize_user("name.with.dot").unwrap_err();
        assert!(format!("{err}").contains("[A-Za-z0-9_-]"));
    }

    #[test]
    fn post_meta_includes_subreddit_score_and_comments() {
        let meta = post_meta(&Post {
            title: Some("Hello".into()),
            score: Some(12),
            num_comments: Some(3),
            subreddit: Some("rust".into()),
            permalink: None,
            url: None,
        });
        assert_eq!(meta, "r/rust · 12↑ 3c");
    }

    #[test]
    fn render_comments_linked_text_block_uses_permalink() {
        let body = render_comments(
            &[Comment {
                body: Some("Hello from a comment".into()),
                score: Some(9),
                subreddit: Some("rust".into()),
                permalink: Some("/r/rust/comments/abc/post/def/".into()),
            }],
            80,
            Shape::LinkedTextBlock,
        );
        let Body::LinkedTextBlock(data) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(data.items[0].text, "r/rust · 9↑  Hello from a comment");
        assert_eq!(
            data.items[0].url.as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc/post/def/")
        );
    }

    #[test]
    fn truncate_adds_ellipsis_when_needed() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
    }

    #[test]
    fn listing_deserializes_post_shape() {
        let raw = r#"{
          "data": {
            "children": [
              {
                "data": {
                  "title": "Post",
                  "score": 42,
                  "num_comments": 7,
                  "subreddit": "rust",
                  "permalink": "/r/rust/comments/abc/post/",
                  "url": "https://example.com/post"
                }
              }
            ]
          }
        }"#;
        let parsed: Listing<Post> = serde_json::from_str(raw).unwrap();
        let first = &parsed.data.children[0].data;
        assert_eq!(first.title.as_deref(), Some("Post"));
        assert_eq!(first.score, Some(42));
        assert_eq!(first.num_comments, Some(7));
        assert_eq!(first.subreddit.as_deref(), Some("rust"));
    }

    #[test]
    fn normalized_count_clamps_to_bounds() {
        assert_eq!(normalized_count(None), DEFAULT_COUNT as usize);
        assert_eq!(normalized_count(Some(0)), MIN_COUNT as usize);
        assert_eq!(normalized_count(Some(999)), MAX_COUNT as usize);
    }

    #[test]
    fn listing_type_controls_period_usage() {
        assert!(SubredditListingType::Top.needs_period());
        assert!(!SubredditListingType::Hot.needs_period());
        assert!(!SubredditListingType::New.needs_period());
        assert!(!SubredditListingType::Rising.needs_period());
        assert_eq!(SubredditListingType::Top.path(), "top");
        assert_eq!(SubredditListingType::Hot.path(), "hot");
        assert_eq!(SubredditListingType::New.path(), "new");
        assert_eq!(SubredditListingType::Rising.path(), "rising");
    }

    #[test]
    fn period_query_covers_all_variants() {
        assert_eq!(Period::Hour.as_query(), "hour");
        assert_eq!(Period::Day.as_query(), "day");
        assert_eq!(Period::Week.as_query(), "week");
        assert_eq!(Period::Month.as_query(), "month");
        assert_eq!(Period::Year.as_query(), "year");
        assert_eq!(Period::All.as_query(), "all");
    }

    #[test]
    fn network_unavailable_body_matches_requested_shape() {
        let linked = network_unavailable_body(Shape::LinkedTextBlock, "reddit 403 Forbidden");
        let Body::LinkedTextBlock(linked) = linked else {
            panic!("expected linked_text_block");
        };
        assert_eq!(linked.items.len(), 1);
        assert!(linked.items[0].text.contains("reddit 403 Forbidden"));

        let entries = network_unavailable_body(Shape::Entries, "reddit timeout");
        let Body::Entries(entries) = entries else {
            panic!("expected entries");
        };
        assert_eq!(entries.items[0].key, "reddit unavailable");
        assert_eq!(entries.items[0].value.as_deref(), Some("reddit timeout"));
    }

    #[test]
    fn post_link_prefers_external_url_then_permalink() {
        let external = Post {
            title: Some("t".into()),
            score: Some(1),
            num_comments: Some(1),
            subreddit: Some("rust".into()),
            permalink: Some("/r/rust/comments/abc/t/".into()),
            url: Some("https://example.com/post".into()),
        };
        assert_eq!(
            post_link(&external).as_deref(),
            Some("https://example.com/post")
        );

        let permalink_only = Post {
            title: Some("t".into()),
            score: Some(1),
            num_comments: Some(1),
            subreddit: Some("rust".into()),
            permalink: Some("/r/rust/comments/abc/t/".into()),
            url: Some("not-a-http-url".into()),
        };
        assert_eq!(
            post_link(&permalink_only).as_deref(),
            Some("https://www.reddit.com/r/rust/comments/abc/t/")
        );
    }

    #[test]
    fn comment_meta_handles_missing_subreddit() {
        let comment = Comment {
            body: Some("hello".into()),
            score: Some(7),
            subreddit: None,
            permalink: None,
        };
        assert_eq!(comment_meta(&comment), "7↑");
    }

    #[test]
    fn normalize_subreddit_rejects_overlength() {
        let long = "a".repeat(22);
        let err = normalize_subreddit(&long).unwrap_err();
        assert!(format!("{err}").contains("too long"));
    }
}
