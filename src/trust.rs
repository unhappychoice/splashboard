#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::tmp_path_for;
use crate::config::{DashboardConfig, WidgetConfig};
use crate::fetcher::{Registry, Safety};
use crate::payload::{Body, Payload, TextBlockData};

const TRUST_ALL_ENV: &str = "SPLASHBOARD_TRUST_ALL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    /// Global config, baked-in default, or `SPLASHBOARD_TRUST_ALL` set. No store entry needed.
    ImplicitlyTrusted,
    /// Local config whose hash matches an entry in the trust store.
    Trusted,
    /// Local config with no entry, or an entry whose hash no longer matches (config edited).
    Untrusted,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    #[serde(default, rename = "entry")]
    entries: Vec<TrustEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustEntry {
    pub path: PathBuf,
    pub sha256: String,
}

impl TrustStore {
    /// Loads the on-disk store, or returns an empty store if it's missing or corrupt. A broken
    /// trust store must never cause the splash to misbehave; the worst case is that the user
    /// needs to re-run `splashboard trust`.
    pub fn load() -> Self {
        store_path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomic write so a concurrent invocation or crashed write never leaves the store empty
    /// (which would silently untrust every entry). Same tmp+rename idiom as the payload cache.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = store_path() else {
            return Err(io::Error::other("could not resolve trust store path"));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string(self).map_err(io::Error::other)?;
        let tmp = tmp_path_for(&path);
        std::fs::write(&tmp, body)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Caller is expected to have already read the dashboard bytes and hashed them (via
    /// [`load_dashboard_and_hash`]). Re-hashing here would open a TOCTOU window: a trusted file
    /// could be swapped for an attacker-crafted one between the caller's load and our hash.
    ///
    /// `ident = None` signals a HOME-backed source (settings, `home.dashboard.toml`, or
    /// `project.dashboard.toml`) — always implicitly trusted because the user owns HOME.
    pub fn decide(&self, ident: Option<(&Path, &str)>) -> TrustDecision {
        if trust_all_override() {
            return TrustDecision::ImplicitlyTrusted;
        }
        let Some((path, hash)) = ident else {
            return TrustDecision::ImplicitlyTrusted;
        };
        let canon = canonicalize_or(path);
        for entry in &self.entries {
            if canonicalize_or(&entry.path) == canon && entry.sha256 == hash {
                return TrustDecision::Trusted;
            }
        }
        TrustDecision::Untrusted
    }

    pub fn trust(&mut self, path: &Path, hash: String) -> io::Result<()> {
        let canon = path.canonicalize()?;
        self.entries.retain(|e| canonicalize_or(&e.path) != canon);
        self.entries.push(TrustEntry {
            path: canon,
            sha256: hash,
        });
        self.save()
    }

    pub fn revoke(&mut self, path: &Path) -> io::Result<bool> {
        let canon = canonicalize_or(path);
        let before = self.entries.len();
        self.entries.retain(|e| canonicalize_or(&e.path) != canon);
        let removed = self.entries.len() < before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn list(&self) -> &[TrustEntry] {
        &self.entries
    }
}

/// Reads the dashboard bytes once and returns both the parsed `DashboardConfig` and its
/// sha256. All trust-sensitive callers MUST use this instead of `DashboardConfig::parse` +
/// `hash_file` — two separate reads would let an attacker swap the file between the load and
/// the hash.
pub fn load_dashboard_and_hash(path: &Path) -> io::Result<(DashboardConfig, String)> {
    let bytes = std::fs::read(path)?;
    let hash = hash_bytes(&bytes);
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let dashboard = DashboardConfig::parse(text).map_err(io::Error::other)?;
    Ok((dashboard, hash))
}

pub fn hash_file(path: &Path) -> io::Result<String> {
    Ok(hash_bytes(&std::fs::read(path)?))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Partitions widgets into (fetchable, gated). Widgets whose fetcher requires trust
/// (Network or Exec) are moved to `gated` when the config is untrusted; callers should render
/// `requires_trust_placeholder()` for those slots instead of fetching.
pub fn partition_by_trust(
    widgets: &[WidgetConfig],
    registry: &Registry,
    decision: TrustDecision,
) -> (Vec<WidgetConfig>, Vec<WidgetConfig>) {
    if !matches!(decision, TrustDecision::Untrusted) {
        return (widgets.to_vec(), Vec::new());
    }
    widgets
        .iter()
        .cloned()
        .partition(|w| match registry.get(&w.fetcher) {
            Some(f) => matches!(f.safety(), Safety::Safe),
            None => true,
        })
}

pub fn requires_trust_placeholder() -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body: Body::TextBlock(TextBlockData {
            lines: vec!["🔒 requires trust".into(), "run `splashboard trust`".into()],
        }),
    }
}

fn trust_all_override() -> bool {
    matches!(
        std::env::var(TRUST_ALL_ENV).ok().as_deref(),
        Some("1") | Some("true")
    )
}

fn canonicalize_or(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn store_path() -> Option<PathBuf> {
    crate::paths::trust_store_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetcher::{FetchContext, FetchError, Fetcher, RealtimeFetcher};
    use crate::render::Shape;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct FakeFetcher {
        name: &'static str,
        safety: Safety,
    }

    #[async_trait]
    impl Fetcher for FakeFetcher {
        fn name(&self) -> &str {
            self.name
        }
        fn safety(&self) -> Safety {
            self.safety
        }
        fn shapes(&self) -> &[Shape] {
            &[Shape::Text]
        }
        async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
            unimplemented!("fake fetcher not invoked in tests")
        }
    }

    struct FakeRealtime {
        name: &'static str,
        safety: Safety,
    }

    impl RealtimeFetcher for FakeRealtime {
        fn name(&self) -> &str {
            self.name
        }
        fn safety(&self) -> Safety {
            self.safety
        }
        fn shapes(&self) -> &[Shape] {
            &[Shape::Text]
        }
        fn compute(&self, _: &FetchContext) -> Payload {
            unimplemented!("fake realtime not invoked in tests")
        }
    }

    fn registry_with(fetchers: &[(&'static str, Safety)]) -> Registry {
        let mut r = Registry::default();
        for (name, safety) in fetchers {
            r.register(Arc::new(FakeFetcher {
                name,
                safety: *safety,
            }));
        }
        r
    }

    fn widget(id: &str, fetcher: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: fetcher.into(),
            ..Default::default()
        }
    }

    #[test]
    fn hash_file_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, "hello").unwrap();
        let a = hash_file(&p).unwrap();
        let b = hash_file(&p).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn hash_file_changes_when_contents_change() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, "hello").unwrap();
        let before = hash_file(&p).unwrap();
        std::fs::write(&p, "world").unwrap();
        let after = hash_file(&p).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn load_dashboard_and_hash_returns_matching_hash() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".splashboard.toml");
        std::fs::write(&p, "").unwrap();
        let (_cfg, hash) = load_dashboard_and_hash(&p).unwrap();
        assert_eq!(hash, hash_file(&p).unwrap());
    }

    #[test]
    fn decide_none_ident_is_implicitly_trusted() {
        let store = TrustStore::default();
        assert_eq!(store.decide(None), TrustDecision::ImplicitlyTrusted);
    }

    #[test]
    fn decide_unlisted_local_config_is_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".splashboard.toml");
        std::fs::write(&p, "").unwrap();
        let hash = hash_file(&p).unwrap();
        let store = TrustStore::default();
        assert_eq!(store.decide(Some((&p, &hash))), TrustDecision::Untrusted);
    }

    #[test]
    fn decide_trusted_after_explicit_trust() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".splashboard.toml");
        std::fs::write(&p, "hello").unwrap();
        let hash = hash_file(&p).unwrap();
        let canon = p.canonicalize().unwrap();
        let store = TrustStore {
            entries: vec![TrustEntry {
                path: canon,
                sha256: hash.clone(),
            }],
        };
        assert_eq!(store.decide(Some((&p, &hash))), TrustDecision::Trusted);
    }

    #[test]
    fn decide_rejects_mismatched_hash_even_if_path_matches() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".splashboard.toml");
        std::fs::write(&p, "original").unwrap();
        let stored_hash = hash_file(&p).unwrap();
        let store = TrustStore {
            entries: vec![TrustEntry {
                path: p.canonicalize().unwrap(),
                sha256: stored_hash,
            }],
        };
        // Simulate attacker swapping the file: caller passes a hash that doesn't match the entry.
        let attacker_hash = "f".repeat(64);
        assert_eq!(
            store.decide(Some((&p, &attacker_hash))),
            TrustDecision::Untrusted
        );
    }

    #[test]
    fn home_backed_source_is_implicitly_trusted_via_none_ident() {
        // HOME-backed sources (settings.toml, home.dashboard.toml, project.dashboard.toml) are
        // implicitly trusted by the caller passing `ident = None`. A local dashboard with the
        // same on-disk bytes is NOT auto-trusted — the gate is caller-driven, not path-driven.
        let store = TrustStore::default();
        assert_eq!(store.decide(None), TrustDecision::ImplicitlyTrusted);
    }

    #[test]
    fn partition_keeps_everything_when_trusted() {
        let registry = registry_with(&[
            ("static", Safety::Safe),
            ("github_prs", Safety::Network),
            ("plugin", Safety::Exec),
        ]);
        let widgets = vec![
            widget("g", "static"),
            widget("p", "github_prs"),
            widget("x", "plugin"),
        ];
        let (fetchable, gated) = partition_by_trust(&widgets, &registry, TrustDecision::Trusted);
        assert_eq!(fetchable.len(), 3);
        assert!(gated.is_empty());
    }

    #[test]
    fn partition_gates_network_and_exec_when_untrusted() {
        let registry = registry_with(&[
            ("static", Safety::Safe),
            ("github_prs", Safety::Network),
            ("plugin", Safety::Exec),
        ]);
        let widgets = vec![
            widget("g", "static"),
            widget("p", "github_prs"),
            widget("x", "plugin"),
        ];
        let (fetchable, gated) = partition_by_trust(&widgets, &registry, TrustDecision::Untrusted);
        let fetchable_ids: Vec<String> = fetchable.iter().map(|w| w.id.clone()).collect();
        let gated_ids: Vec<String> = gated.iter().map(|w| w.id.clone()).collect();
        assert_eq!(fetchable_ids, vec!["g".to_string()]);
        assert!(gated_ids.iter().any(|s| s == "p"));
        assert!(gated_ids.iter().any(|s| s == "x"));
    }

    #[test]
    fn partition_treats_realtime_safe_fetcher_as_fetchable_when_untrusted() {
        // A realtime fetcher exposes safety via the unified Registry entry; a Safe realtime
        // fetcher must not be gated just because it isn't the async Fetcher kind.
        let mut registry = Registry::default();
        registry.register_realtime(Arc::new(FakeRealtime {
            name: "clock",
            safety: Safety::Safe,
        }));
        let widgets = vec![widget("c", "clock")];
        let (fetchable, gated) = partition_by_trust(&widgets, &registry, TrustDecision::Untrusted);
        assert_eq!(fetchable.len(), 1);
        assert!(gated.is_empty());
    }

    #[test]
    fn partition_keeps_unknown_fetchers_in_fetchable_when_untrusted() {
        let registry = registry_with(&[("static", Safety::Safe)]);
        let widgets = vec![widget("x", "mystery")];
        let (fetchable, gated) = partition_by_trust(&widgets, &registry, TrustDecision::Untrusted);
        assert_eq!(fetchable.len(), 1);
        assert!(gated.is_empty());
    }

    #[test]
    fn trust_store_round_trips_via_toml() {
        let store = TrustStore {
            entries: vec![TrustEntry {
                path: PathBuf::from("/tmp/x/.splashboard.toml"),
                sha256: "a".repeat(64),
            }],
        };
        let s = toml::to_string(&store).unwrap();
        let back: TrustStore = toml::from_str(&s).unwrap();
        assert_eq!(back.entries, store.entries);
    }

    #[test]
    fn placeholder_has_lock_icon_in_first_line() {
        let p = requires_trust_placeholder();
        match p.body {
            Body::TextBlock(t) => {
                assert!(t.lines[0].contains("🔒"));
                assert!(t.lines[1].contains("splashboard trust"));
            }
            _ => panic!("expected text_block body"),
        }
    }
}
