//! `hackernews_user` — profile rollup for an HN account via `/user/{login}.json`. Same read,
//! three shapes:
//!
//! - `Text` — `"@login · Npt karma"` for hero / subtitle bands.
//! - `Entries` — login / karma / created / submissions count, key/value rows.
//! - `TextBlock` — bio block (login / about / created / karma) for sidebar use.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::fetcher::github::common::{cache_key, parse_options, payload};
use crate::fetcher::{FetchContext, FetchError, Fetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, LinkedLine, LinkedTextBlockData, Payload};
use crate::render::Shape;
use crate::samples;

use super::client::{API_BASE, HN_USER_URL, get};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::LinkedTextBlock,
    Shape::TextBlock,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "user",
    type_hint: "string (HN login)",
    required: true,
    default: None,
    description: "Hacker News login to fetch (e.g. `\"pg\"`).",
}];

/// Profile rollup for a single Hacker News account.
pub struct HackernewsUserFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
}

#[async_trait]
impl Fetcher for HackernewsUserFetcher {
    fn name(&self) -> &str {
        "hackernews_user"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
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
            Shape::Text => samples::text("@pg · 156000pt karma"),
            Shape::Entries => samples::entries(&[
                ("login", "pg"),
                ("karma", "156000"),
                ("created", "2006-10-09"),
                ("submissions", "1234"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "@pg",
                "156000pt karma",
                "joined 2006-10-09",
                "Paul Graham, Y Combinator co-founder.",
            ]),
            Shape::LinkedTextBlock => samples::linked_text_block(&[(
                "@pg · 156000pt karma",
                Some("https://news.ycombinator.com/user?id=pg"),
            )]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let login = opts
            .user
            .ok_or_else(|| FetchError::Failed("hackernews_user requires `user` option".into()))?;
        let info: UserInfo = get(&format!("{API_BASE}/user/{login}.json")).await?;
        Ok(payload(render_body(
            &info,
            ctx.shape.unwrap_or(Shape::Entries),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    id: String,
    #[serde(default)]
    karma: u64,
    /// Unix seconds.
    #[serde(default)]
    created: i64,
    #[serde(default)]
    about: Option<String>,
    #[serde(default)]
    submitted: Vec<u64>,
}

fn render_body(info: &UserInfo, shape: Shape) -> Body {
    match shape {
        Shape::Text => crate::payload::Body::Text(crate::payload::TextData {
            value: format!("@{} · {}pt karma", info.id, info.karma),
        }),
        Shape::TextBlock => crate::payload::Body::TextBlock(crate::payload::TextBlockData {
            lines: text_block_lines(info),
        }),
        Shape::LinkedTextBlock => Body::LinkedTextBlock(LinkedTextBlockData {
            items: vec![LinkedLine {
                text: format!("@{} · {}pt karma", info.id, info.karma),
                url: Some(format!("{HN_USER_URL}{}", info.id)),
            }],
        }),
        _ => Body::Entries(EntriesData {
            items: entries(info),
        }),
    }
}

fn entries(info: &UserInfo) -> Vec<Entry> {
    let mut items = vec![
        kv("login", info.id.clone()),
        kv("karma", info.karma.to_string()),
        kv("created", iso_date(info.created)),
        kv("submissions", info.submitted.len().to_string()),
    ];
    if let Some(about) = info.about.as_deref().filter(|s| !s.is_empty()) {
        items.push(kv("about", strip_tags(about)));
    }
    items
}

fn text_block_lines(info: &UserInfo) -> Vec<String> {
    let mut lines = vec![
        format!("@{}", info.id),
        format!("{}pt karma", info.karma),
        format!("joined {}", iso_date(info.created)),
    ];
    if let Some(about) = info.about.as_deref().filter(|s| !s.is_empty()) {
        lines.push(strip_tags(about));
    }
    lines
}

fn kv(key: &str, value: String) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value),
        status: None,
    }
}

fn iso_date(unix_seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(unix_seconds, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// HN's `about` field embeds raw HTML (`<p>`, `<a>`, `&#x2F;`). Strip the markup conservatively
/// so the rollup stays text-only — replacing `<br>` / `<p>` with spaces and dropping the rest.
/// This isn't a full HTML parser; it's a "best-effort make it readable on a splash" pass.
fn strip_tags(raw: &str) -> String {
    let with_breaks = raw.replace("<p>", " ").replace("<br>", " ");
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        match (ch, in_tag) {
            ('<', _) => in_tag = true,
            ('>', _) => in_tag = false,
            (c, false) => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#x2F;", "/")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(about: Option<&str>) -> UserInfo {
        UserInfo {
            id: "pg".into(),
            karma: 156000,
            created: 1_160_418_092, // 2006-10-09
            about: about.map(String::from),
            submitted: vec![1, 2, 3, 4, 5],
        }
    }

    #[test]
    fn options_require_user() {
        let opts: Options = toml::from_str("user = \"pg\"").unwrap();
        assert_eq!(opts.user.as_deref(), Some("pg"));
    }

    #[test]
    fn text_shape_combines_login_and_karma() {
        let body = render_body(&info(None), Shape::Text);
        let crate::payload::Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.contains("@pg"));
        assert!(t.value.contains("156000pt"));
    }

    #[test]
    fn entries_shape_includes_canonical_rows() {
        let body = render_body(&info(None), Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        let keys: Vec<_> = e.items.iter().map(|e| e.key.as_str()).collect();
        assert!(keys.contains(&"login"));
        assert!(keys.contains(&"karma"));
        assert!(keys.contains(&"created"));
        assert!(keys.contains(&"submissions"));
    }

    #[test]
    fn entries_shape_appends_about_when_present() {
        let body = render_body(&info(Some("Paul.")), Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert!(e.items.iter().any(|x| x.key == "about"));
    }

    #[test]
    fn linked_text_block_shape_links_to_user_page() {
        let body = render_body(&info(None), Shape::LinkedTextBlock);
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked text block");
        };
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://news.ycombinator.com/user?id=pg")
        );
    }

    #[test]
    fn iso_date_formats_unix_seconds() {
        assert_eq!(iso_date(1_160_418_092), "2006-10-09");
    }

    #[test]
    fn iso_date_falls_back_to_empty_for_invalid_timestamp() {
        assert_eq!(iso_date(i64::MAX), "");
    }

    #[test]
    fn strip_tags_drops_html_and_decodes_entities() {
        let out = strip_tags("<p>Hello &amp; <a href=\"/x\">world</a></p>");
        assert_eq!(out, "Hello & world");
    }

    #[test]
    fn strip_tags_collapses_whitespace_runs() {
        // <p> becomes a single space, multiple internal spaces get folded by split_whitespace.
        assert_eq!(strip_tags("a<p><p><p>b"), "a b");
        assert_eq!(strip_tags("a    b\n\nc"), "a b c");
    }

    #[test]
    fn decode_entities_handles_canonical_set() {
        assert_eq!(
            decode_entities("&amp;&lt;&gt;&quot;&#x27;&#x2F;&#39;"),
            "&<>\"'/'"
        );
    }

    #[test]
    fn text_block_lines_omit_about_when_blank() {
        let lines = text_block_lines(&info(None));
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "@pg");
        assert!(lines[1].contains("156000pt"));
        assert!(lines[2].starts_with("joined "));
    }

    #[test]
    fn text_block_lines_append_about_when_present() {
        let lines = text_block_lines(&info(Some("<p>Founder</p>")));
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[3], "Founder");
    }

    #[test]
    fn entries_uses_submission_count_not_full_array() {
        let body = render_body(&info(None), Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        let submissions = e.items.iter().find(|x| x.key == "submissions").unwrap();
        assert_eq!(submissions.value.as_deref(), Some("5"));
    }

    #[test]
    fn empty_about_is_treated_as_missing() {
        let body = render_body(&info(Some("")), Shape::Entries);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert!(e.items.iter().all(|x| x.key != "about"));
    }

    #[test]
    fn user_info_deserializes_minimal_payload() {
        let raw = r#"{"id":"pg"}"#;
        let info: UserInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.id, "pg");
        assert_eq!(info.karma, 0);
        assert_eq!(info.created, 0);
        assert!(info.about.is_none());
        assert!(info.submitted.is_empty());
    }
}
