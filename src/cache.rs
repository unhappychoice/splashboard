#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::payload::Payload;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

const STALE_LOCK_THRESHOLD: Duration = Duration::from_secs(30);

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
        let tmp = tmp_path_for(&path);
        let json = serde_json::to_string(entry).map_err(io_err)?;
        ensure_parent(&path)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn path_for(&self, widget_id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", sanitize(widget_id)))
    }

    /// Non-blocking: returns None if another process already holds the lock for this key. Locks
    /// older than `STALE_LOCK_THRESHOLD` are stolen (assumed crashed).
    pub fn try_lock(&self, key: &str) -> Option<Lock> {
        Lock::try_acquire(&self.dir, key)
    }
}

pub struct Lock {
    path: PathBuf,
}

impl Lock {
    pub fn try_acquire(dir: &Path, key: &str) -> Option<Self> {
        Self::try_acquire_with(dir, key, STALE_LOCK_THRESHOLD)
    }

    fn try_acquire_with(dir: &Path, key: &str, stale_after: Duration) -> Option<Self> {
        let path = lock_path_for(dir, key);
        match create_exclusive(&path) {
            Ok(()) => Some(Self { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if is_lock_stale(&path, stale_after) {
                    let _ = std::fs::remove_file(&path);
                    create_exclusive(&path).ok().map(|_| Self { path })
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn lock_path_for(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{}.lock", sanitize(key)))
}

fn create_exclusive(path: &Path) -> std::io::Result<()> {
    ensure_parent(path)?;
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    writeln!(f, "{}", std::process::id())?;
    Ok(())
}

fn is_lock_stale(path: &Path, threshold: Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(mtime) = meta.modified() else {
        return false;
    };
    mtime.elapsed().map(|d| d > threshold).unwrap_or(false)
}

/// Builds a per-invocation unique tmp path next to `final_path`. Pid keeps it distinct across
/// processes; the atomic counter keeps it distinct across concurrent calls within one process.
/// Exported so other modules (e.g. trust store) can share the write-tmp-then-rename idiom.
pub(crate) fn tmp_path_for(final_path: &Path) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_path.with_file_name(format!("{file_name}.{pid}.{seq}.tmp"))
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
    use crate::payload::{Body, LinesData};

    fn sample() -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData {
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

    #[test]
    fn tmp_path_is_unique_across_calls() {
        let final_path = Path::new("/tmp/cache/foo.json");
        let a = tmp_path_for(final_path);
        let b = tmp_path_for(final_path);
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains("foo.json."));
        assert!(a.to_string_lossy().ends_with(".tmp"));
    }

    #[test]
    fn lock_is_exclusive_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let held = Lock::try_acquire(dir.path(), "k").expect("first acquire succeeds");
        assert!(
            Lock::try_acquire(dir.path(), "k").is_none(),
            "second acquire must fail while held"
        );
        drop(held);
        assert!(
            Lock::try_acquire(dir.path(), "k").is_some(),
            "acquire succeeds after drop"
        );
    }

    #[test]
    fn stale_lock_is_stolen() {
        let dir = tempfile::tempdir().unwrap();
        // Plant a fake lock file.
        let path = lock_path_for(dir.path(), "k");
        std::fs::write(&path, "999999\n").unwrap();
        std::thread::sleep(Duration::from_millis(5));
        let lock = Lock::try_acquire_with(dir.path(), "k", Duration::from_millis(1));
        assert!(lock.is_some(), "stale lock should be stolen");
    }

    #[test]
    fn fresh_lock_is_not_stolen() {
        let dir = tempfile::tempdir().unwrap();
        let _held = Lock::try_acquire(dir.path(), "k").unwrap();
        let lock = Lock::try_acquire_with(dir.path(), "k", Duration::from_secs(60));
        assert!(lock.is_none(), "fresh lock must not be stolen");
    }

    #[test]
    fn lock_drop_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _lock = Lock::try_acquire(dir.path(), "k").unwrap();
            assert!(lock_path_for(dir.path(), "k").exists());
        }
        assert!(!lock_path_for(dir.path(), "k").exists());
    }

    #[test]
    fn different_keys_do_not_block_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let _a = Lock::try_acquire(dir.path(), "a").unwrap();
        let _b = Lock::try_acquire(dir.path(), "b").unwrap();
    }

    #[test]
    fn concurrent_stores_leave_valid_final_file() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempfile::tempdir().unwrap();
        let cache = Arc::new(Cache::open(dir.path().to_path_buf()).unwrap());
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let cache = Arc::clone(&cache);
                thread::spawn(move || {
                    let entry = CacheEntry::new(sample(), 100 + i);
                    cache.store("shared", &entry).unwrap();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let loaded = cache.load("shared").expect("final file must parse");
        assert!(loaded.ttl_seconds >= 100);
        // No leftover tmp files should remain in the cache dir.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover tmp files: {leftovers:?}");
    }
}
