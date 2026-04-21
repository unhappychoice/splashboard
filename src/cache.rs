#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::payload::Payload;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheEntry {
    pub refreshed_at: u64,
    pub ttl_seconds: u64,
    pub payload: Payload,
}

impl CacheEntry {
    pub fn new(payload: Payload, ttl_seconds: u64) -> Self {
        Self {
            refreshed_at: now_secs(),
            ttl_seconds,
            payload,
        }
    }

    pub fn is_fresh(&self) -> bool {
        age_of(self.refreshed_at) < self.ttl_seconds
    }
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn open_default() -> Option<Self> {
        let dir = dirs::cache_dir()?.join("splashboard");
        Self::open(dir)
    }

    pub fn open(dir: PathBuf) -> Option<Self> {
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self { dir })
    }

    pub fn load(&self, widget_id: &str) -> Option<CacheEntry> {
        let data = std::fs::read_to_string(self.path_for(widget_id)).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn store(&self, widget_id: &str, entry: &CacheEntry) -> std::io::Result<()> {
        let path = self.path_for(widget_id);
        let tmp = path.with_extension("tmp");
        let json = serde_json::to_string(entry).map_err(io_err)?;
        ensure_parent(&path)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn path_for(&self, widget_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", sanitize(widget_id)))
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn ensure_parent(path: &Path) -> std::io::Result<()> {
    match path.parent() {
        Some(p) => std::fs::create_dir_all(p),
        None => Ok(()),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn age_of(past: u64) -> u64 {
    now_secs().saturating_sub(past)
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Body, TextData};

    fn sample() -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                lines: vec!["hi".into()],
            }),
        }
    }

    #[test]
    fn entry_is_fresh_when_inside_ttl() {
        let entry = CacheEntry::new(sample(), 60);
        assert!(entry.is_fresh());
    }

    #[test]
    fn entry_is_stale_when_ttl_zero() {
        let entry = CacheEntry::new(sample(), 0);
        assert!(!entry.is_fresh());
    }

    #[test]
    fn entry_is_stale_when_timestamp_is_ancient() {
        let entry = CacheEntry {
            refreshed_at: 0,
            ttl_seconds: 60,
            payload: sample(),
        };
        assert!(!entry.is_fresh());
    }

    #[test]
    fn store_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let entry = CacheEntry::new(sample(), 60);
        cache.store("greeting", &entry).unwrap();
        let loaded = cache.load("greeting").unwrap();
        assert_eq!(loaded, entry);
    }

    #[test]
    fn load_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        assert!(cache.load("absent").is_none());
    }

    #[test]
    fn store_overwrites_previous_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        cache.store("k", &CacheEntry::new(sample(), 10)).unwrap();
        let updated = CacheEntry::new(sample(), 999);
        cache.store("k", &updated).unwrap();
        assert_eq!(cache.load("k").unwrap(), updated);
    }

    #[test]
    fn path_for_sanitizes_unsafe_chars() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let path = cache.path_for("../evil/id");
        assert!(path.starts_with(dir.path()));
        assert!(!path.to_string_lossy().contains(".."));
    }

    #[test]
    fn load_returns_none_for_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        std::fs::write(cache.path_for("bad"), "not-json").unwrap();
        assert!(cache.load("bad").is_none());
    }
}
