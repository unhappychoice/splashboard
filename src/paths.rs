#![allow(dead_code)]

use std::path::PathBuf;

/// All splashboard state lives under a single dotted directory in the user's home:
/// `$HOME/.splashboard/`. Keeps settings, dashboards, trust store, cache, and ReadStore data
/// in one place — same pattern as per-directory `.splashboard/` configs. Overridable via the
/// `SPLASHBOARD_HOME` env var for tests, CI, or power users who want to relocate.
///
/// Rationale over the platform-specific `dirs` crate: for a CLI tool, `~/Library/...` on
/// macOS is awkward (Unix-flavored OS, dotfile conventions are the norm for terminal
/// tools). `$HOME/.splashboard/` works the same on Linux, macOS, and Windows, and mirrors
/// the per-directory convention users already know.
pub fn splashboard_home() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SPLASHBOARD_HOME") {
        return Some(PathBuf::from(p));
    }
    dirs::home_dir().map(|h| h.join(".splashboard"))
}

/// User preferences (`[general]` + `[theme]`). Always lives in HOME — per-dir overrides are
/// out of scope for 0.x.
pub fn settings_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("settings.toml"))
}

/// Default dashboard when no per-dir dashboard applies and the CWD isn't a project root.
pub fn home_dashboard_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("home.dashboard.toml"))
}

/// Default project dashboard used when the CWD is a git repo root without a per-dir dashboard.
pub fn project_dashboard_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("project.dashboard.toml"))
}

pub fn trust_store_path() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("trust.toml"))
}

pub fn cache_dir() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("cache"))
}

pub fn read_store_dir() -> Option<PathBuf> {
    splashboard_home().map(|d| d.join("store"))
}

/// Shared mutex guarding any test that mutates `SPLASHBOARD_HOME`. Without this, parallel
/// tests in different modules can see each other's temporary values through the process-wide
/// env var. Exposed at crate level so other test modules (`fetcher::read_store`, etc.) can
/// lock the same mutex.
#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// All subpaths flow through `splashboard_home()` so overriding the env var relocates
    /// every splashboard-owned file atomically — important for test isolation and for
    /// power users who want to `SPLASHBOARD_HOME=/opt/splashboard` ship the whole state
    /// directory elsewhere.
    #[test]
    fn env_var_overrides_home() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        let previous = std::env::var("SPLASHBOARD_HOME").ok();
        // SAFETY: mutating process env in tests. Scoped to restore on exit; mutex above
        // ensures no other test races with us on this env var.
        unsafe {
            std::env::set_var("SPLASHBOARD_HOME", &path);
        }
        assert_eq!(splashboard_home(), Some(path.clone()));
        assert_eq!(settings_path(), Some(path.join("settings.toml")));
        assert_eq!(
            home_dashboard_path(),
            Some(path.join("home.dashboard.toml"))
        );
        assert_eq!(
            project_dashboard_path(),
            Some(path.join("project.dashboard.toml"))
        );
        assert_eq!(trust_store_path(), Some(path.join("trust.toml")));
        assert_eq!(cache_dir(), Some(path.join("cache")));
        assert_eq!(read_store_dir(), Some(path.join("store")));
        unsafe {
            match previous {
                Some(v) => std::env::set_var("SPLASHBOARD_HOME", v),
                None => std::env::remove_var("SPLASHBOARD_HOME"),
            }
        }
    }
}
