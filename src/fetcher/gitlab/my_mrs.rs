//! `gitlab_my_mrs` — open merge requests authored by the authenticated GitLab user across
//! every project. Uses `/api/v4/merge_requests?scope=created_by_me&state=opened`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::fetcher::forge_items::{self, LIST_SHAPES};
use crate::options::OptionSchema;
use crate::payload::{Body, Payload};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::{default_host, rest_get};
use super::common::{cache_key, parse_options, payload, validate_host};
use super::items::{GitlabIssueLike, sample_rows, to_forge_rows};

const DEFAULT_LIMIT: u32 = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "host",
        type_hint: "hostname",
        required: false,
        default: Some("gitlab.com"),
        description: "GitLab instance host. Use this for self-hosted GitLab.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Maximum number of MRs to show.",
    },
];

pub struct GitlabMyMrs;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GitlabMyMrs {
    fn name(&self) -> &str {
        "gitlab_my_mrs"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Open merge requests authored by the authenticated GitLab user across every project. Use `gitlab_repo_mrs` instead to list MRs against one specific project regardless of who opened them."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        LIST_SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let host = host_for_key(ctx);
        cache_key(self.name(), ctx, &host)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let rows = sample_rows(&[
            (
                "g/splashboard!54",
                "feat(docs): generate widget catalogue",
                Some("https://gitlab.com/g/splashboard/-/merge_requests/54"),
                7,
                1_774_000_000,
            ),
            (
                "g/splashboard!51",
                "feat(fetcher): split clock options",
                Some("https://gitlab.com/g/splashboard/-/merge_requests/51"),
                2,
                1_773_800_000,
            ),
        ]);
        forge_items::dispatch_sample(&rows, shape, "open MR", "open MRs")
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let host = resolve_host(opts.host.as_deref())?;
        let limit = opts.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 30);
        let path = format!(
            "/merge_requests?scope=created_by_me&state=opened&per_page={limit}&order_by=updated_at"
        );
        let items: Vec<GitlabIssueLike> = rest_get(&host, &path).await?;
        let rows = to_forge_rows(&items, true);
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let body = forge_items::dispatch_rows_async(rows, shape, "open MR", "open MRs").await;
        Ok(payload(body))
    }
}

pub(super) fn resolve_host(explicit: Option<&str>) -> Result<String, FetchError> {
    let host = explicit.unwrap_or(default_host());
    validate_host(host)
        .map(String::from)
        .map_err(FetchError::Failed)
}

fn host_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or(default_host())
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::payload::{BadgeData, Status};

    #[test]
    fn options_deserialize_both_fields() {
        let raw: toml::Value = toml::from_str("host = \"git.example.org\"\nlimit = 5").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.host.as_deref(), Some("git.example.org"));
        assert_eq!(opts.limit, Some(5));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("limit = 5\nbogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn shapes_table_matches_shared_list() {
        let fetcher = GitlabMyMrs;
        assert_eq!(fetcher.shapes(), LIST_SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(fetcher.safety(), Safety::Safe);
    }

    #[test]
    fn sample_body_covers_every_list_shape_plus_badge() {
        let fetcher = GitlabMyMrs;
        for shape in LIST_SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}"
            );
        }
        // Shapes outside the catalogued list (e.g. Ratio) return None instead of an empty body
        // so docs generation never accidentally promotes them.
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn sample_badge_starts_calm_when_count_is_low_but_warm_when_open() {
        let fetcher = GitlabMyMrs;
        let Some(Body::Badge(BadgeData { status, label })) = fetcher.sample_body(Shape::Badge)
        else {
            panic!("expected badge");
        };
        assert_eq!(status, Status::Warn);
        assert_eq!(label, "2 open MRs");
    }

    #[test]
    fn host_for_key_falls_back_to_default() {
        assert_eq!(host_for_key(&FetchContext::default()), "gitlab.com");
    }

    #[test]
    fn host_for_key_reads_explicit_host_option() {
        let ctx = FetchContext {
            options: Some(toml::from_str("host = \"git.example.org\"").unwrap()),
            ..Default::default()
        };
        assert_eq!(host_for_key(&ctx), "git.example.org");
    }

    #[test]
    fn cache_key_changes_with_host_option() {
        let fetcher = GitlabMyMrs;
        let a = fetcher.cache_key(&FetchContext::default());
        let b = fetcher.cache_key(&FetchContext {
            options: Some(toml::from_str("host = \"git.example.org\"").unwrap()),
            ..Default::default()
        });
        assert_ne!(a, b);
        assert!(a.starts_with("gitlab_my_mrs-"));
    }

    #[test]
    fn resolve_host_validates_input() {
        assert_eq!(resolve_host(None).unwrap(), "gitlab.com");
        assert!(resolve_host(Some("https://gitlab.com")).is_err());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_host_before_network_request() {
        let err = GitlabMyMrs
            .fetch(&FetchContext {
                options: Some(toml::from_str("host = \"https://gitlab.com\"").unwrap()),
                timeout: Duration::from_secs(1),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(m) if m.contains("invalid gitlab host")
        ));
    }
}
