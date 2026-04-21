#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{WidgetConfig, default_global_path};
use crate::fetcher::{Registry, Safety};
use crate::payload::{Body, Payload, TextData};

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
    /// Loads the on-disk store, or returns an empty store if it's missing or corrupt. We
    /// deliberately do not surface corruption errors: a broken trust store must not cause the
    /// splash to misbehave. Worst case: user re-runs `splashboard trust`.
    pub fn load() -> Self {
        store_path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let Some(path) = store_path() else {
            return Err(io::Error::other("could not resolve trust store path"));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml = toml::to_string(self).map_err(io::Error::other)?;
        std::fs::write(&path, toml)
    }

    pub fn decide(&self, config_path: Option<&Path>) -> TrustDecision {
        if trust_all_override() {
            return TrustDecision::ImplicitlyTrusted;
        }
        let Some(path) = config_path else {
            return TrustDecision::ImplicitlyTrusted;
        };
        if is_global(path) {
            return TrustDecision::ImplicitlyTrusted;
        }
        let Ok(hash) = hash_file(path) else {
            return TrustDecision::Untrusted;
        };
        let canon = canonicalize_or(path);
        for entry in &self.entries {
            if canonicalize_or(&entry.path) == canon && entry.sha256 == hash {
                return TrustDecision::Trusted;
            }
        }
        TrustDecision::Untrusted
    }

    pub fn trust(&mut self, path: &Path) -> io::Result<()> {
        let hash = hash_file(path)?;
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
        body: Body::Text(TextData {
            lines: vec!["🔒 requires trust".into(), "run `splashboard trust`".into()],
        }),
    }
}

pub fn hash_file(path: &Path) -> io::Result<String> {
    let data = std::fs::read(path)?;
    let digest = Sha256::digest(&data);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn trust_all_override() -> bool {
    matches!(
        std::env::var(TRUST_ALL_ENV).ok().as_deref(),
        Some("1") | Some("true")
    )
}

fn is_global(path: &Path) -> bool {
    let Some(global) = default_global_path() else {
        return false;
    };
    canonicalize_or(path) == canonicalize_or(&global)
}

fn canonicalize_or(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("splashboard").join("trusted.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RenderType;
    use crate::fetcher::{FetchContext, FetchError, Fetcher};
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
        async fn fetch(&self, _: &FetchContext) -> Result<Payload, FetchError> {
            unimplemented!("fake fetcher not invoked in tests")
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
            render: RenderType::Text,
            format: None,
            refresh_interval: None,
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
    fn decide_none_path_is_implicitly_trusted() {
        let store = TrustStore::default();
        assert_eq!(store.decide(None), TrustDecision::ImplicitlyTrusted);
    }

    #[test]
    fn decide_unlisted_local_config_is_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".splashboard.toml");
        std::fs::write(&p, "").unwrap();
        let store = TrustStore::default();
        assert_eq!(store.decide(Some(&p)), TrustDecision::Untrusted);
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
                sha256: hash,
            }],
        };
        assert_eq!(store.decide(Some(&p)), TrustDecision::Trusted);
    }

    #[test]
    fn decide_detects_modification_after_trust() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".splashboard.toml");
        std::fs::write(&p, "original").unwrap();
        let hash = hash_file(&p).unwrap();
        let store = TrustStore {
            entries: vec![TrustEntry {
                path: p.canonicalize().unwrap(),
                sha256: hash,
            }],
        };
        std::fs::write(&p, "tampered").unwrap();
        assert_eq!(store.decide(Some(&p)), TrustDecision::Untrusted);
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
    fn partition_keeps_unknown_fetchers_in_fetchable_when_untrusted() {
        // Unknown fetchers will simply be skipped at fetch time; gating them away isn't useful
        // since they can't actually reach anything anyway.
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
            Body::Text(t) => {
                assert!(t.lines[0].contains("🔒"));
                assert!(t.lines[1].contains("splashboard trust"));
            }
            _ => panic!("expected text body"),
        }
    }
}
