//! `github_user` — profile data for a GitHub login. Calls `/users/{login}` and exposes
//! the fields most useful for a hero / subtitle line: display name, bio, location,
//! join year, follower counts. Used by the `home_github` preset to replace an
//! opaque `static` subtitle with a live pull.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{cache_key, parse_options, payload};

// TextBlock is listed first so the default renderer (text_plain accepts both Text and
// TextBlock) picks the 3-line profile block — that's the header-band use case. Users
// who want the tight `@login · location` one-liner explicitly pin `render = "text_plain"`
// on a fetcher whose shapes-intersection lands on Text (or swap to a Text-only renderer).
const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Text, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "user",
    type_hint: "string (github login)",
    required: false,
    default: Some("$GITHUB_USER env var"),
    description: "GitHub login to fetch. Falls back to the `GITHUB_USER` env var when omitted.",
}];

pub struct GithubUser;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub user: Option<String>,
}

#[async_trait]
impl Fetcher for GithubUser {
    fn name(&self) -> &str {
        "github_user"
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
        let extra = user_for_key(ctx);
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("@unhappychoice · Tokyo, Japan"),
            Shape::TextBlock => samples::text_block(&[
                "Yuji Ueki",
                "Terminal splash renderer maintainer",
                "Tokyo, Japan · member since 2013",
            ]),
            Shape::Entries => samples::entries(&[
                ("name", "Yuji Ueki"),
                ("bio", "Terminal splash renderer maintainer"),
                ("location", "Tokyo, Japan"),
                ("company", ""),
                ("member_since", "2013"),
                ("followers", "420"),
                ("following", "69"),
                ("public_repos", "48"),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let user = resolve_user(opts.user.as_deref()).map_err(FetchError::Failed)?;
        let info: UserInfo = rest_get(&format!("/users/{user}")).await?;
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: one_liner(&info),
            }),
            Shape::Entries => Body::Entries(EntriesData {
                items: entries(&info),
            }),
            _ => Body::TextBlock(TextBlockData {
                lines: text_block_lines(&info),
            }),
        };
        Ok(payload(body))
    }
}

/// `@login · Location · Company` — drops any empty segment so the line stays tight.
fn one_liner(info: &UserInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("@{}", info.login));
    if let Some(loc) = nonempty(&info.location) {
        parts.push(loc.clone());
    }
    if let Some(co) = nonempty(&info.company) {
        parts.push(co.clone());
    }
    parts.join(" · ")
}

/// 3-line block for subtitle use: display name / bio / location + join year. Empty
/// fields collapse so users without a bio don't see a stranded blank line.
fn text_block_lines(info: &UserInfo) -> Vec<String> {
    let name = nonempty(&info.name).cloned().unwrap_or(info.login.clone());
    let mut lines = vec![name];
    if let Some(bio) = nonempty(&info.bio) {
        lines.push(bio.clone());
    }
    let mut tail = Vec::new();
    if let Some(loc) = nonempty(&info.location) {
        tail.push(loc.clone());
    }
    if let Some(year) = join_year(&info.created_at) {
        tail.push(format!("member since {year}"));
    }
    if !tail.is_empty() {
        lines.push(tail.join(" · "));
    }
    lines
}

fn entries(info: &UserInfo) -> Vec<Entry> {
    vec![
        entry("name", nonempty(&info.name).cloned().unwrap_or_default()),
        entry("bio", nonempty(&info.bio).cloned().unwrap_or_default()),
        entry(
            "location",
            nonempty(&info.location).cloned().unwrap_or_default(),
        ),
        entry(
            "company",
            nonempty(&info.company).cloned().unwrap_or_default(),
        ),
        entry(
            "member_since",
            join_year(&info.created_at).unwrap_or_default(),
        ),
        entry("followers", info.followers.to_string()),
        entry("following", info.following.to_string()),
        entry("public_repos", info.public_repos.to_string()),
    ]
}

fn entry(key: &str, value: String) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value),
        status: None,
    }
}

fn nonempty(s: &Option<String>) -> Option<&String> {
    s.as_ref().filter(|v| !v.is_empty())
}

fn join_year(created_at: &Option<String>) -> Option<String> {
    created_at
        .as_deref()
        .and_then(|s| s.get(0..4))
        .map(String::from)
}

fn resolve_user(explicit: Option<&str>) -> Result<String, String> {
    if let Some(u) = explicit.filter(|s| !s.is_empty()) {
        return Ok(u.into());
    }
    if let Ok(u) = std::env::var("GITHUB_USER")
        && !u.is_empty()
    {
        return Ok(u);
    }
    Err("set `user = \"<login>\"` or the GITHUB_USER env var".into())
}

fn user_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("user"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| std::env::var("GITHUB_USER").ok())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    bio: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    followers: u64,
    #[serde(default)]
    following: u64,
    #[serde(default)]
    public_repos: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(
        name: Option<&str>,
        bio: Option<&str>,
        location: Option<&str>,
        company: Option<&str>,
        created_at: Option<&str>,
    ) -> UserInfo {
        UserInfo {
            login: "unhappychoice".into(),
            name: name.map(String::from),
            bio: bio.map(String::from),
            location: location.map(String::from),
            company: company.map(String::from),
            created_at: created_at.map(String::from),
            followers: 420,
            following: 69,
            public_repos: 48,
        }
    }

    #[test]
    fn one_liner_joins_nonempty_fields() {
        let i = info(None, None, Some("Tokyo"), Some("Acme"), None);
        assert_eq!(one_liner(&i), "@unhappychoice · Tokyo · Acme");
    }

    #[test]
    fn one_liner_drops_empty_segments() {
        let i = info(None, None, None, None, None);
        assert_eq!(one_liner(&i), "@unhappychoice");
    }

    #[test]
    fn text_block_uses_name_then_bio_then_tail() {
        let i = info(
            Some("Yuji Ueki"),
            Some("TUI hacker"),
            Some("Tokyo"),
            None,
            Some("2013-04-12T13:57:32Z"),
        );
        assert_eq!(
            text_block_lines(&i),
            vec![
                "Yuji Ueki".to_string(),
                "TUI hacker".into(),
                "Tokyo · member since 2013".into(),
            ]
        );
    }

    #[test]
    fn text_block_falls_back_to_login_when_name_missing() {
        let i = info(None, None, None, None, None);
        assert_eq!(text_block_lines(&i), vec!["unhappychoice".to_string()]);
    }

    #[test]
    fn join_year_parses_iso_timestamp() {
        assert_eq!(
            join_year(&Some("2013-04-12T13:57:32Z".into())),
            Some("2013".into())
        );
    }

    #[test]
    fn join_year_none_when_empty() {
        assert_eq!(join_year(&None), None);
    }
}
