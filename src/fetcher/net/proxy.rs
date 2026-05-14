//! `net_proxy` — the `$*_PROXY` environment, i.e. whether this shell routes through a proxy.
//!
//! The one realtime fetcher in the family: pure `std::env` reads, infallible, no I/O.

use crate::payload::{
    BadgeData, Body, EntriesData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, payload};

const SHAPES: &[Shape] = &[
    Shape::Entries,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Badge,
];

const DIRECT: &str = "(direct)";
const UNSET: &str = "(unset)";

pub struct NetProxy;

/// Resolved proxy environment. Each field prefers the lowercase variable name (`http_proxy`)
/// over the uppercase one (`HTTP_PROXY`), matching the de-facto precedence most CLIs use.
struct ProxyState {
    http: Option<String>,
    https: Option<String>,
    no_proxy: Option<String>,
}

impl ProxyState {
    fn is_proxied(&self) -> bool {
        self.http.is_some() || self.https.is_some()
    }

    /// The proxy that actually carries traffic — HTTPS first since nearly everything is TLS now.
    fn active(&self) -> Option<&str> {
        self.https.as_deref().or(self.http.as_deref())
    }
}

impl RealtimeFetcher for NetProxy {
    fn name(&self) -> &str {
        "net_proxy"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The shell's `$*_PROXY` environment. `Entries` (default) / `TextBlock` / `MarkdownTextBlock` roll up `http_proxy` / `https_proxy` / `no_proxy`; `Text` shows the proxy that actually carries traffic (HTTPS first), or `(direct)`; `Badge` is a proxied / direct pill."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_state(), shape)
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let state = read_proxy_state();
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        payload(body_for_shape(&state, shape).unwrap_or_else(|| entries_body(&state)))
    }
}

fn read_proxy_state() -> ProxyState {
    ProxyState {
        http: env_either("http_proxy", "HTTP_PROXY"),
        https: env_either("https_proxy", "HTTPS_PROXY"),
        no_proxy: env_either("no_proxy", "NO_PROXY"),
    }
}

fn env_either(lower: &str, upper: &str) -> Option<String> {
    std::env::var(lower)
        .ok()
        .or_else(|| std::env::var(upper).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn body_for_shape(state: &ProxyState, shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::Entries => entries_body(state),
        Shape::Text => Body::Text(TextData {
            value: state.active().unwrap_or(DIRECT).to_string(),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: rows(state).map(|(k, v)| format!("{k}  {v}")).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: rows(state)
                .map(|(k, v)| format!("- **{k}** {v}"))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Badge => Body::Badge(proxy_badge(state)),
        _ => return None,
    })
}

fn entries_body(state: &ProxyState) -> Body {
    Body::Entries(EntriesData {
        items: rows(state).map(|(k, v)| entry(k, &v)).collect(),
    })
}

fn rows(state: &ProxyState) -> impl Iterator<Item = (&'static str, String)> {
    [
        ("http_proxy", state.http.clone()),
        ("https_proxy", state.https.clone()),
        ("no_proxy", state.no_proxy.clone()),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.unwrap_or_else(|| UNSET.to_string())))
}

/// `Warn`, not `Error` — being proxied isn't broken, but it's worth a glance-level flag since it
/// silently changes where every request goes.
fn proxy_badge(state: &ProxyState) -> BadgeData {
    if state.is_proxied() {
        BadgeData {
            status: Status::Warn,
            label: "proxied".into(),
        }
    } else {
        BadgeData {
            status: Status::Ok,
            label: "direct".into(),
        }
    }
}

fn sample_state() -> ProxyState {
    ProxyState {
        http: Some("http://proxy.corp:8080".into()),
        https: Some("http://proxy.corp:8080".into()),
        no_proxy: Some("localhost,127.0.0.1,.internal".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::system::test_helpers::{EnvGuard, ctx_with_shape};

    const PROXY_KEYS: &[&str] = &[
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "no_proxy",
        "NO_PROXY",
    ];

    fn clear() -> Vec<(&'static str, Option<&'static str>)> {
        PROXY_KEYS.iter().map(|k| (*k, None)).collect()
    }

    #[test]
    fn body_for_shape_covers_every_supported_shape() {
        let state = sample_state();
        for &shape in SHAPES {
            let body = body_for_shape(&state, shape).unwrap();
            assert_eq!(crate::render::shape_of(&body), shape);
        }
        assert!(body_for_shape(&state, Shape::Ratio).is_none());
        assert!(body_for_shape(&state, Shape::Bars).is_none());
    }

    #[test]
    fn active_prefers_https_then_http_then_direct() {
        let both = sample_state();
        assert_eq!(both.active(), Some("http://proxy.corp:8080"));
        let http_only = ProxyState {
            http: Some("http://h:1".into()),
            https: None,
            no_proxy: None,
        };
        assert_eq!(http_only.active(), Some("http://h:1"));
        let none = ProxyState {
            http: None,
            https: None,
            no_proxy: None,
        };
        assert_eq!(none.active(), None);
        assert!(!none.is_proxied());
    }

    #[test]
    fn entries_fall_back_to_unset_marker() {
        let partial = ProxyState {
            http: None,
            https: Some("http://h:1".into()),
            no_proxy: None,
        };
        let Body::Entries(d) = entries_body(&partial) else {
            panic!("expected entries");
        };
        assert_eq!(d.items.len(), 3);
        assert_eq!(d.items[0].value.as_deref(), Some(UNSET));
        assert_eq!(d.items[1].value.as_deref(), Some("http://h:1"));
    }

    #[test]
    fn badge_distinguishes_proxied_from_direct() {
        assert_eq!(proxy_badge(&sample_state()).status, Status::Warn);
        let direct = ProxyState {
            http: None,
            https: None,
            no_proxy: Some("localhost".into()),
        };
        // no_proxy alone is not "proxied".
        assert_eq!(proxy_badge(&direct).status, Status::Ok);
        assert_eq!(proxy_badge(&direct).label, "direct");
    }

    #[test]
    fn compute_reads_live_env_lowercase_first() {
        let mut env = clear();
        env.push(("https_proxy", Some("http://low:8080")));
        env.push(("HTTPS_PROXY", Some("http://up:9090")));
        let _guard = EnvGuard::set(&env);
        let Body::Text(t) = NetProxy.compute(&ctx_with_shape(Some(Shape::Text))).body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "http://low:8080");
    }

    #[test]
    fn compute_reports_direct_when_env_is_clear() {
        let _guard = EnvGuard::set(&clear());
        let Body::Badge(b) = NetProxy.compute(&ctx_with_shape(Some(Shape::Badge))).body else {
            panic!("expected badge");
        };
        assert_eq!(b.status, Status::Ok);
        assert_eq!(b.label, "direct");
    }

    #[test]
    fn fetcher_contract_metadata() {
        assert_eq!(NetProxy.name(), "net_proxy");
        assert_eq!(NetProxy.safety(), Safety::Safe);
        assert_eq!(NetProxy.default_shape(), Shape::Entries);
        assert!(NetProxy.option_schemas().is_empty());
        for &shape in SHAPES {
            assert!(NetProxy.sample_body(shape).is_some());
        }
        assert!(NetProxy.sample_body(Shape::Ratio).is_none());
    }
}
