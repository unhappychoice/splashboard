#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::payload::Payload;

pub mod stub;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Safety {
    Safe,
    Network,
    Exec,
}

#[derive(Debug, Clone)]
pub enum FetchError {
    Unknown(String),
    Failed(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "unknown fetcher: {name}"),
            Self::Failed(msg) => write!(f, "fetch failed: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

#[derive(Debug, Clone)]
pub struct FetchContext {
    pub widget_id: String,
    pub format: Option<String>,
    pub timeout: Duration,
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    /// Identity of this fetcher-invocation for caching. Two calls that share a key see the same
    /// payload. Default covers the common case (`name + format`); fetchers whose output depends on
    /// more (cwd, repo, URL) override this to mix those in.
    fn cache_key(&self, ctx: &FetchContext) -> String {
        default_cache_key(self.name(), ctx)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError>;
}

pub fn default_cache_key(name: &str, ctx: &FetchContext) -> String {
    use sha2::{Digest, Sha256};
    let raw = format!("{}|{}", name, ctx.format.as_deref().unwrap_or(""));
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

#[derive(Default, Clone)]
pub struct Registry {
    fetchers: HashMap<String, Arc<dyn Fetcher>>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        for f in stub::builtins() {
            r.register(f);
        }
        r
    }

    pub fn register(&mut self, f: Arc<dyn Fetcher>) {
        self.fetchers.insert(f.name().to_string(), f);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Fetcher>> {
        self.fetchers.get(name).cloned()
    }

    pub fn names(&self) -> Vec<&str> {
        self.fetchers.keys().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Body, TextData};

    struct Dummy(&'static str, Safety);

    #[async_trait]
    impl Fetcher for Dummy {
        fn name(&self) -> &str {
            self.0
        }
        fn safety(&self) -> Safety {
            self.1
        }
        async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
            Ok(Payload {
                icon: None,
                status: None,
                format: None,
                body: Body::Text(TextData { lines: vec![] }),
            })
        }
    }

    #[test]
    fn registry_register_and_get() {
        let mut r = Registry::default();
        r.register(Arc::new(Dummy("x", Safety::Safe)));
        assert!(r.get("x").is_some());
        assert!(r.get("missing").is_none());
    }

    #[test]
    fn with_builtins_registers_static_fetcher() {
        let r = Registry::with_builtins();
        assert!(r.get("static").is_some());
    }

    #[tokio::test]
    async fn dummy_fetcher_round_trip() {
        let f: Arc<dyn Fetcher> = Arc::new(Dummy("d", Safety::Network));
        let ctx = FetchContext {
            widget_id: "w".into(),
            format: None,
            timeout: Duration::from_secs(1),
        };
        let p = f.fetch(&ctx).await.unwrap();
        assert!(matches!(p.body, Body::Text(_)));
        assert_eq!(f.safety(), Safety::Network);
    }

    fn ctx_with(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn cache_key_is_stable_for_same_inputs() {
        let a = default_cache_key("static", &ctx_with(Some("Hi!")));
        let b = default_cache_key("static", &ctx_with(Some("Hi!")));
        assert_eq!(a, b);
    }

    #[test]
    fn cache_key_differs_across_fetchers() {
        let a = default_cache_key("static", &ctx_with(None));
        let b = default_cache_key("clock", &ctx_with(None));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_differs_when_format_differs() {
        let a = default_cache_key("static", &ctx_with(Some("Hi!")));
        let b = default_cache_key("static", &ctx_with(Some("Bye!")));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_ignores_widget_id_and_timeout() {
        // Two different widgets that point at the same fetcher with the same params share cache.
        let mut a = ctx_with(Some("x"));
        let mut b = ctx_with(Some("x"));
        a.widget_id = "greeting_one".into();
        b.widget_id = "greeting_two".into();
        a.timeout = Duration::from_secs(1);
        b.timeout = Duration::from_secs(99);
        assert_eq!(
            default_cache_key("static", &a),
            default_cache_key("static", &b)
        );
    }

    #[test]
    fn cache_key_is_prefixed_with_fetcher_name() {
        let k = default_cache_key("static", &ctx_with(None));
        assert!(k.starts_with("static-"));
    }
}
