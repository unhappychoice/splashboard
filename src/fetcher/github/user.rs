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
use super::client::{resolve_authenticated_user, rest_get};
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
    default: Some("authenticated token user"),
    description: "GitHub login to fetch. Falls back to the `GITHUB_USER` env var, then to the login that owns `GH_TOKEN` / `GITHUB_TOKEN`.",
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
    fn description(&self) -> &'static str {
        "GitHub profile data for a user (display name, bio, location, join year, follower counts), aimed at the hero / subtitle band of a home preset. Pair with `github_avatar` for the matching image."
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
        let user = resolve_user(opts.user.as_deref())
            .await
            .map_err(FetchError::Failed)?;
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

/// Up-to-4-line block for subtitle use: `@login` / display name / bio / location + join year.
/// Empty fields collapse so users without a bio don't see a stranded blank line.
fn text_block_lines(info: &UserInfo) -> Vec<String> {
    let mut lines = vec![format!("@{}", info.login)];
    if let Some(name) = nonempty(&info.name) {
        lines.push(name.clone());
    }
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

async fn resolve_user(explicit: Option<&str>) -> Result<String, String> {
    if let Some(u) = explicit.filter(|s| !s.is_empty()) {
        return Ok(u.into());
    }
    if let Ok(u) = std::env::var("GITHUB_USER")
        && !u.is_empty()
    {
        return Ok(u);
    }
    resolve_authenticated_user()
        .await
        .map_err(|e| format!("resolve github user: {e}"))
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
    use std::future::Future;
    use std::time::Duration;

    use super::*;

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    match value {
                        Some(current) => unsafe { std::env::set_var(key, current) },
                        None => unsafe { std::env::remove_var(key) },
                    }
                    (*key, previous)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.restore.iter().for_each(|(key, value)| match value {
                Some(previous) => unsafe { std::env::set_var(key, previous) },
                None => unsafe { std::env::remove_var(key) },
            });
        }
    }

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

    fn ctx(options: Option<&str>, shape: Option<Shape>, format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "github-user".into(),
            format: format.map(str::to_string),
            timeout: Duration::from_secs(1),
            file_format: None,
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            timezone: None,
            locale: None,
        }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn fetcher_metadata_and_samples_cover_supported_shapes() {
        let fetcher = GithubUser;
        assert_eq!(fetcher.name(), "github_user");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("GitHub profile data"));
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.option_schemas().len(), 1);
        assert_eq!(fetcher.option_schemas()[0].name, "user");
        assert_eq!(
            fetcher.option_schemas()[0].default,
            Some("authenticated token user")
        );

        let Some(Body::Text(text)) = fetcher.sample_body(Shape::Text) else {
            panic!("expected text sample");
        };
        assert_eq!(text.value, "@unhappychoice · Tokyo, Japan");

        let Some(Body::TextBlock(block)) = fetcher.sample_body(Shape::TextBlock) else {
            panic!("expected text block sample");
        };
        assert_eq!(block.lines[0], "Yuji Ueki");
        assert_eq!(block.lines[2], "Tokyo, Japan · member since 2013");

        let Some(Body::Entries(entries)) = fetcher.sample_body(Shape::Entries) else {
            panic!("expected entries sample");
        };
        assert_eq!(entries.items[0].key, "name");
        assert_eq!(entries.items[5].value.as_deref(), Some("420"));
        assert!(fetcher.sample_body(Shape::Bars).is_none());
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
    fn text_block_leads_with_login_then_name_bio_tail() {
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
                "@unhappychoice".to_string(),
                "Yuji Ueki".into(),
                "TUI hacker".into(),
                "Tokyo · member since 2013".into(),
            ]
        );
    }

    #[test]
    fn text_block_collapses_to_just_login_when_all_else_missing() {
        let i = info(None, None, None, None, None);
        assert_eq!(text_block_lines(&i), vec!["@unhappychoice".to_string()]);
    }

    #[test]
    fn entries_fill_missing_optional_fields_with_empty_strings() {
        let rows = entries(&info(
            None,
            None,
            Some("Tokyo"),
            None,
            Some("2013-04-12T13:57:32Z"),
        ));
        assert_eq!(rows[0].value.as_deref(), Some(""));
        assert_eq!(rows[2].value.as_deref(), Some("Tokyo"));
        assert_eq!(rows[4].value.as_deref(), Some("2013"));
        assert_eq!(rows[7].value.as_deref(), Some("48"));
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

    #[test]
    fn join_year_rejects_short_values() {
        assert_eq!(join_year(&Some("202".into())), None);
    }

    #[test]
    fn user_for_key_prefers_options_over_env() {
        let _guard = EnvGuard::set(&[("GITHUB_USER", Some("from-env"))]);
        assert_eq!(
            user_for_key(&ctx(Some("user = \"from-options\""), None, None)),
            "from-options"
        );
    }

    #[test]
    fn user_for_key_falls_back_to_env() {
        let _guard = EnvGuard::set(&[("GITHUB_USER", Some("from-env"))]);
        assert_eq!(
            user_for_key(&ctx(None, Some(Shape::Text), None)),
            "from-env"
        );
    }

    #[test]
    fn cache_key_is_stable_for_equivalent_context_and_changes_with_user() {
        let fetcher = GithubUser;
        let alice = ctx(
            Some("user = \"alice\""),
            Some(Shape::Entries),
            Some("compact"),
        );
        let alice_again = ctx(
            Some("user = \"alice\""),
            Some(Shape::Entries),
            Some("compact"),
        );
        let bob = ctx(
            Some("user = \"bob\""),
            Some(Shape::Entries),
            Some("compact"),
        );

        assert_eq!(fetcher.cache_key(&alice), fetcher.cache_key(&alice_again));
        assert_ne!(fetcher.cache_key(&alice), fetcher.cache_key(&bob));
    }

    #[test]
    fn resolve_user_prefers_explicit_value() {
        let _guard = EnvGuard::set(&[("GITHUB_USER", Some("from-env"))]);
        assert_eq!(
            run_async(resolve_user(Some("explicit"))).unwrap(),
            "explicit"
        );
    }

    #[test]
    fn resolve_user_falls_back_to_env() {
        let _guard = EnvGuard::set(&[
            ("GITHUB_USER", Some("from-env")),
            ("GH_TOKEN", None),
            ("GITHUB_TOKEN", None),
        ]);
        crate::fetcher::github::client::clear_authenticated_user_cache();
        assert_eq!(run_async(resolve_user(None)).unwrap(), "from-env");
    }

    #[test]
    fn resolve_user_reports_missing_auth_without_sources() {
        let _guard = EnvGuard::set(&[
            ("GITHUB_USER", None),
            ("GH_TOKEN", None),
            ("GITHUB_TOKEN", None),
        ]);
        crate::fetcher::github::client::clear_authenticated_user_cache();
        let err = run_async(resolve_user(None)).expect_err("expected missing auth");
        assert_eq!(
            err,
            "resolve github user: fetch failed: GH_TOKEN / GITHUB_TOKEN not set"
        );
    }

    #[test]
    fn fetch_rejects_invalid_options_before_resolving_user() {
        let err = run_async(GithubUser.fetch(&ctx(
            Some("user = \"octocat\"\nbogus = true"),
            Some(Shape::Text),
            None,
        )))
        .expect_err("invalid options should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert!(message.contains("invalid options"));
    }

    #[test]
    fn fetch_without_user_or_token_surfaces_resolution_error() {
        let _guard = EnvGuard::set(&[
            ("GITHUB_USER", None),
            ("GH_TOKEN", None),
            ("GITHUB_TOKEN", None),
        ]);
        crate::fetcher::github::client::clear_authenticated_user_cache();
        let err = run_async(GithubUser.fetch(&ctx(None, Some(Shape::TextBlock), None)))
            .expect_err("missing user sources should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(
            message,
            "resolve github user: fetch failed: GH_TOKEN / GITHUB_TOKEN not set"
        );
    }

    #[test]
    fn fetch_with_explicit_user_surfaces_auth_error_before_network() {
        let _guard = EnvGuard::set(&[
            ("GITHUB_USER", None),
            ("GH_TOKEN", None),
            ("GITHUB_TOKEN", None),
        ]);
        crate::fetcher::github::client::clear_authenticated_user_cache();
        let err = run_async(GithubUser.fetch(&ctx(
            Some("user = \"octocat\""),
            Some(Shape::Entries),
            Some("compact"),
        )))
        .expect_err("missing token should fail");
        let FetchError::Failed(message) = err else {
            panic!("expected fetch failure");
        };
        assert_eq!(message, "GH_TOKEN / GITHUB_TOKEN not set");
    }
}
