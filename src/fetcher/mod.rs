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
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError>;
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
}
