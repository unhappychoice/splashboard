//! `linear_*` fetcher family — Linear issues, notifications, current-cycle progress.
//!
//! Every fetcher in this family targets the fixed host `api.linear.app` (see `client::API_URL`),
//! so the personal API key (`options.token` or `LINEAR_TOKEN` env) can never be redirected to
//! an attacker-controlled origin — Safety::Safe.
//!
//! OAuth flow is intentionally not supported: splashboard's startup window has no place to host
//! a callback. Personal API keys (`lin_api_*`) are sent as-is in the `Authorization` header by
//! [`client::graphql_query`].

mod client;
pub mod cycle;
pub mod issues;
pub mod notifications;

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::fetcher::{FetchContext, Fetcher};

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        issues::fetcher(),
        notifications::fetcher(),
        cycle::fetcher(),
    ]
}

/// Cache key for any `linear_*` fetcher. Mixes shape, format, options blob, and a token-scope
/// prefix so two accounts sharing a `$HOME/.splashboard/cache` directory can't observe each
/// other's payloads.
pub(crate) fn cache_key(name: &str, ctx: &FetchContext) -> String {
    let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("");
    let format = ctx.format.as_deref().unwrap_or("");
    let opts_token = ctx
        .options
        .as_ref()
        .and_then(|v| v.get("token"))
        .and_then(|v| v.as_str());
    let extra = client::cache_extra(opts_token, ctx.options.as_ref());
    let raw = format!("{name}|{shape}|{format}|{extra}");
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Shape;

    fn ctx_with(opts: Option<toml::Value>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            options: opts,
            shape: Some(Shape::LinkedTextBlock),
            ..Default::default()
        }
    }

    #[test]
    fn cache_key_is_prefixed_with_fetcher_name() {
        let k = cache_key("linear_issues", &ctx_with(None));
        assert!(k.starts_with("linear_issues-"));
    }

    #[test]
    fn cache_key_partitions_per_token() {
        let opts_a: toml::Value = toml::from_str("token = \"tok-A\"").unwrap();
        let opts_b: toml::Value = toml::from_str("token = \"tok-B\"").unwrap();
        let a = cache_key("linear_issues", &ctx_with(Some(opts_a)));
        let b = cache_key("linear_issues", &ctx_with(Some(opts_b)));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_partitions_per_shape() {
        let mut a = ctx_with(None);
        let mut b = ctx_with(None);
        a.shape = Some(Shape::LinkedTextBlock);
        b.shape = Some(Shape::Bars);
        assert_ne!(
            cache_key("linear_issues", &a),
            cache_key("linear_issues", &b)
        );
    }
}
