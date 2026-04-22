#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::options::OptionSchema;
use crate::payload::{Body, LinesData, Payload};
use crate::render::Shape;
use crate::samples;

pub mod clock;
pub mod git;
pub mod github;
pub mod read_store;
pub mod static_text;
pub mod system;

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

#[derive(Debug, Clone, Default)]
pub struct FetchContext {
    pub widget_id: String,
    pub format: Option<String>,
    pub timeout: Duration,
    /// read_store file encoding — "json" / "toml" / "text". Set by config; ignored by
    /// fetchers that don't read files.
    pub file_format: Option<String>,
    /// Target shape the fetcher should emit. Derived at the runtime boundary from the
    /// widget's renderer (renderers are single-shape today) or the fetcher's
    /// [`Fetcher::default_shape`] when no renderer is specified. Multi-shape fetchers branch
    /// on this; single-shape fetchers ignore it.
    pub shape: Option<Shape>,
    /// Fetcher-specific options passed through from `[widget.options]`. Each fetcher
    /// deserializes its own typed view; unknown keys are ignored.
    pub options: Option<toml::Value>,
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    /// Shapes this fetcher can emit. Single-shape fetchers (static_text, disk) return one
    /// variant; fetchers whose one physical read can be reshaped for different widgets (clock,
    /// read_store) list every variant they accept. Validated against the renderer-derived
    /// shape at the runtime boundary so an unsupported pairing renders a placeholder rather
    /// than silently returning the wrong Body.
    fn shapes(&self) -> &[Shape];
    /// Shape used when config doesn't specify a renderer. Must be an element of `shapes()`.
    /// Defaults to the first entry so simple single-shape fetchers don't need to override.
    fn default_shape(&self) -> Shape {
        self.shapes()[0]
    }
    /// Identity of this fetcher-invocation for caching. Two calls that share a key see the same
    /// payload. Default covers the common case (`name + format`); fetchers whose output depends on
    /// more (cwd, repo, URL) override this to mix those in.
    fn cache_key(&self, ctx: &FetchContext) -> String {
        default_cache_key(self.name(), ctx)
    }
    /// Options this fetcher accepts from `[widget.options]`. Default is an empty slice —
    /// fetchers with no tunables don't need to override. Consumed at docs-generation time by the
    /// `xtask` crate; parsing at runtime is still serde-driven (the schema and the `Options`
    /// struct live side-by-side in the same file).
    fn option_schemas(&self) -> &[OptionSchema] {
        &[]
    }
    /// A representative payload body for the given shape, used to preview what this fetcher
    /// emits without executing it. Default falls back to [`crate::samples::canonical_sample`];
    /// fetchers override to surface something closer to real output (`"main +2 ◆3"` for
    /// `git_status`, `"14:35"` for `clock`). Must be a member of `shapes()`.
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        samples::canonical_sample(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError>;
}

/// Fetchers whose value is intrinsically "right now" — wall clock, instantaneous system counters,
/// countdowns. Contract: computed in <1ms, infallible, no I/O. Bypasses the disk cache entirely
/// and is recomputed by the runtime on every frame. If a prospective fetcher can't meet that
/// contract, it belongs on [`Fetcher`] instead.
pub trait RealtimeFetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    fn shapes(&self) -> &[Shape];
    fn default_shape(&self) -> Shape {
        self.shapes()[0]
    }
    /// See [`Fetcher::option_schemas`]. Default is an empty slice.
    fn option_schemas(&self) -> &[OptionSchema] {
        &[]
    }
    /// See [`Fetcher::sample_body`]. Default falls through to the shape-level canonical sample.
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        samples::canonical_sample(shape)
    }
    fn compute(&self, ctx: &FetchContext) -> Payload;
}

/// Fetcher can't emit the shape the renderer expects. Surfaced as a placeholder so the splash
/// keeps rendering even when the config pairs, say, `fetcher = "disk_usage"` with `render = "calendar"`.
#[derive(Debug, Clone)]
pub struct ShapeMismatch {
    pub fetcher: String,
    pub requested: Shape,
}

/// Structure mirrors [`crate::trust::requires_trust_placeholder`] — a two-line hint so the user
/// can tell what's wrong without opening logs.
pub fn shape_mismatch_placeholder(err: &ShapeMismatch) -> Payload {
    let lines = vec![
        format!("⚠ {} can't emit {}", err.fetcher, err.requested.as_str()),
        "check `render` in config".into(),
    ];
    Payload {
        icon: None,
        status: None,
        format: None,
        body: Body::Lines(LinesData { lines }),
    }
}

pub fn default_cache_key(name: &str, ctx: &FetchContext) -> String {
    use sha2::{Digest, Sha256};
    let raw = format!("{}|{}", name, ctx.format.as_deref().unwrap_or(""));
    let digest = Sha256::digest(raw.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("{name}-{hex}")
}

/// Registry entry — either a cache-backed async fetcher or a recompute-each-frame realtime one.
/// The trust gate and name lookup work against this unified type; runtime unwraps to the specific
/// kind only when it needs the kind-specific behavior (cache I/O vs. per-frame compute).
#[derive(Clone)]
pub enum RegisteredFetcher {
    Cached(Arc<dyn Fetcher>),
    Realtime(Arc<dyn RealtimeFetcher>),
}

impl RegisteredFetcher {
    pub fn name(&self) -> &str {
        match self {
            Self::Cached(f) => f.name(),
            Self::Realtime(f) => f.name(),
        }
    }
    pub fn safety(&self) -> Safety {
        match self {
            Self::Cached(f) => f.safety(),
            Self::Realtime(f) => f.safety(),
        }
    }
    pub fn shapes(&self) -> Vec<Shape> {
        match self {
            Self::Cached(f) => f.shapes().to_vec(),
            Self::Realtime(f) => f.shapes().to_vec(),
        }
    }
    pub fn default_shape(&self) -> Shape {
        match self {
            Self::Cached(f) => f.default_shape(),
            Self::Realtime(f) => f.default_shape(),
        }
    }
    pub fn option_schemas(&self) -> &[OptionSchema] {
        match self {
            Self::Cached(f) => f.option_schemas(),
            Self::Realtime(f) => f.option_schemas(),
        }
    }
    pub fn sample_body(&self, shape: Shape) -> Option<Body> {
        match self {
            Self::Cached(f) => f.sample_body(shape),
            Self::Realtime(f) => f.sample_body(shape),
        }
    }
    pub fn as_cached(&self) -> Option<Arc<dyn Fetcher>> {
        match self {
            Self::Cached(f) => Some(f.clone()),
            Self::Realtime(_) => None,
        }
    }
    pub fn as_realtime(&self) -> Option<Arc<dyn RealtimeFetcher>> {
        match self {
            Self::Realtime(f) => Some(f.clone()),
            Self::Cached(_) => None,
        }
    }
}

#[derive(Default, Clone)]
pub struct Registry {
    fetchers: HashMap<String, RegisteredFetcher>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        let mut r = Self::default();
        r.register(Arc::new(static_text::StaticText));
        r.register(Arc::new(read_store::ReadStoreFetcher));
        for f in clock::realtime_fetchers() {
            r.register_realtime(f);
        }
        for f in system::realtime_fetchers() {
            r.register_realtime(f);
        }
        for f in system::cached_fetchers() {
            r.register(f);
        }
        for f in git::fetchers() {
            r.register(f);
        }
        for f in github::fetchers() {
            r.register(f);
        }
        r
    }

    pub fn register(&mut self, f: Arc<dyn Fetcher>) {
        self.fetchers
            .insert(f.name().to_string(), RegisteredFetcher::Cached(f));
    }

    pub fn register_realtime(&mut self, f: Arc<dyn RealtimeFetcher>) {
        self.fetchers
            .insert(f.name().to_string(), RegisteredFetcher::Realtime(f));
    }

    pub fn get(&self, name: &str) -> Option<RegisteredFetcher> {
        self.fetchers.get(name).cloned()
    }

    pub fn get_cached(&self, name: &str) -> Option<Arc<dyn Fetcher>> {
        self.get(name).and_then(|f| f.as_cached())
    }

    pub fn get_realtime(&self, name: &str) -> Option<Arc<dyn RealtimeFetcher>> {
        self.get(name).and_then(|f| f.as_realtime())
    }

    pub fn names(&self) -> Vec<&str> {
        self.fetchers.keys().map(String::as_str).collect()
    }

    /// All registered fetchers, sorted by name. Useful for deterministic docs generation.
    pub fn sorted(&self) -> Vec<RegisteredFetcher> {
        let mut entries: Vec<_> = self.fetchers.values().cloned().collect();
        entries.sort_by(|a, b| a.name().cmp(b.name()));
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Body, LinesData};

    struct Dummy(&'static str, Safety);

    #[async_trait]
    impl Fetcher for Dummy {
        fn name(&self) -> &str {
            self.0
        }
        fn safety(&self) -> Safety {
            self.1
        }
        fn shapes(&self) -> &[Shape] {
            &[Shape::Lines]
        }
        async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
            Ok(Payload {
                icon: None,
                status: None,
                format: None,
                body: Body::Lines(LinesData { lines: vec![] }),
            })
        }
    }

    struct DummyRealtime(&'static str, Safety);

    impl RealtimeFetcher for DummyRealtime {
        fn name(&self) -> &str {
            self.0
        }
        fn safety(&self) -> Safety {
            self.1
        }
        fn shapes(&self) -> &[Shape] {
            &[Shape::Lines]
        }
        fn compute(&self, _: &FetchContext) -> Payload {
            Payload {
                icon: None,
                status: None,
                format: None,
                body: Body::Lines(LinesData { lines: vec![] }),
            }
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
    fn registry_register_realtime_and_get() {
        let mut r = Registry::default();
        r.register_realtime(Arc::new(DummyRealtime("rt", Safety::Safe)));
        let entry = r.get("rt").expect("must be present");
        assert!(matches!(entry, RegisteredFetcher::Realtime(_)));
        assert_eq!(entry.safety(), Safety::Safe);
    }

    #[test]
    fn registry_name_collision_replaces() {
        // Registering the same name twice — second wins. Prevents both-kinds-of-fetcher confusion.
        let mut r = Registry::default();
        r.register(Arc::new(Dummy("x", Safety::Safe)));
        r.register_realtime(Arc::new(DummyRealtime("x", Safety::Network)));
        let entry = r.get("x").unwrap();
        assert!(matches!(entry, RegisteredFetcher::Realtime(_)));
        assert_eq!(entry.safety(), Safety::Network);
    }

    #[test]
    fn as_cached_and_as_realtime_are_mutually_exclusive() {
        let cached = RegisteredFetcher::Cached(Arc::new(Dummy("c", Safety::Safe)));
        assert!(cached.as_cached().is_some());
        assert!(cached.as_realtime().is_none());
        let rt = RegisteredFetcher::Realtime(Arc::new(DummyRealtime("r", Safety::Safe)));
        assert!(rt.as_realtime().is_some());
        assert!(rt.as_cached().is_none());
    }

    #[test]
    fn with_builtins_registers_static_fetcher() {
        let r = Registry::with_builtins();
        assert!(r.get("static").is_some());
    }

    #[test]
    fn with_builtins_registers_clock_as_realtime() {
        let r = Registry::with_builtins();
        let entry = r.get("clock").expect("clock must be registered");
        assert!(matches!(entry, RegisteredFetcher::Realtime(_)));
    }

    #[tokio::test]
    async fn dummy_fetcher_round_trip() {
        let f: Arc<dyn Fetcher> = Arc::new(Dummy("d", Safety::Network));
        let ctx = FetchContext {
            widget_id: "w".into(),
            format: None,
            timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let p = f.fetch(&ctx).await.unwrap();
        assert!(matches!(p.body, Body::Lines(_)));
        assert_eq!(f.safety(), Safety::Network);
    }

    fn ctx_with(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
            ..Default::default()
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
