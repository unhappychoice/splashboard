//! `tracing` initialization for splashboard.
//!
//! Writes structured log lines to `$HOME/.splashboard/logs/splashboard.log.<date>` via
//! `tracing-appender`'s daily-rotating file appender, keeping the latest 3 files. The
//! `SPLASHBOARD_LOG` env var controls the filter (defaults to `error` so the log stays quiet
//! until something goes wrong). Multiple calls in the same process are a no-op — the first
//! caller wins.
//!
//! Separate from `stderr` / `stdout` on purpose: the main splashboard invocation owns the
//! terminal (inline viewport + prompt handoff), and the daemon runs detached with null stdio.
//! A file appender is the only sink that works in both.

use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;

use crate::paths;

const LOG_FILENAME_PREFIX: &str = "splashboard";
const LOG_FILENAME_SUFFIX: &str = "log";
const MAX_LOG_FILES: usize = 3;
const DEFAULT_FILTER: &str = "error";

/// Holds the `tracing-appender` worker guard for the lifetime of the process. Dropped on
/// process exit — the `WorkerGuard` flushes the non-blocking writer's buffer, so a process
/// crashing mid-fetch still lands its error line in the log.
static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Initializes the tracing subscriber. Safe to call from both the main CLI and the fetch-only
/// daemon; the first caller wins. Silent failure — if the log directory can't be created (no
/// `$HOME`, read-only fs) the subscriber is simply not installed and `tracing::error!` calls
/// become no-ops.
pub fn init() {
    if GUARD.get().is_some() {
        return;
    }
    let Some(dir) = paths::logs_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Some(appender) = build_appender(&dir) else {
        return;
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("SPLASHBOARD_LOG")
        .unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(false);
    // Use `try_init` so a second subscriber install (e.g. tests racing the daemon) doesn't
    // panic; the existing subscriber keeps receiving events.
    if subscriber.try_init().is_ok() {
        let _ = GUARD.set(guard);
    }
}

fn build_appender(dir: &PathBuf) -> Option<tracing_appender::rolling::RollingFileAppender> {
    Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILENAME_PREFIX)
        .filename_suffix(LOG_FILENAME_SUFFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(dir)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::TEST_ENV_LOCK;
    use std::path::Path;
    use std::time::Duration;
    use tracing_appender::non_blocking;
    use tracing_subscriber::fmt::MakeWriter;

    fn restore_env(key: &str, value: Option<String>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    fn wait_for_log_file(dir: &Path) -> bool {
        let mut seen_once = false;
        for _ in 0..40 {
            let any = std::fs::read_dir(dir)
                .ok()
                .into_iter()
                .flat_map(|entries| entries.filter_map(Result::ok))
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(LOG_FILENAME_PREFIX)
                });
            if any && seen_once {
                return true;
            }
            seen_once = any;
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn build_appender_creates_a_file_on_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let appender = build_appender(&dir.path().to_path_buf()).expect("appender builds");
        let (writer, _guard) = non_blocking(appender);
        {
            let mut w = writer.make_writer();
            use std::io::Write;
            writeln!(w, "hello from tests").unwrap();
        }
        drop(_guard);
        assert!(
            wait_for_log_file(dir.path()),
            "log file must be created under {:?}",
            dir.path()
        );
    }

    #[test]
    fn build_appender_rejects_file_path() {
        let dir = tempfile::tempdir().unwrap();
        let occupied = dir.path().join("occupied");
        std::fs::write(&occupied, "not a directory").unwrap();
        assert!(build_appender(&occupied).is_none());
    }

    #[test]
    fn init_skips_invalid_home_then_initializes_once() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let blocked_home = tmp.path().join("blocked-home");
        std::fs::write(&blocked_home, "occupied").unwrap();
        let good_home = tmp.path().join("good-home");
        let later_home = tmp.path().join("later-home");
        let outer_previous_home = std::env::var("SPLASHBOARD_HOME").ok();
        let outer_previous_filter = std::env::var("SPLASHBOARD_LOG").ok();
        unsafe {
            std::env::set_var("SPLASHBOARD_HOME", tmp.path().join("previous-home"));
            std::env::set_var("SPLASHBOARD_LOG", "warn");
        }
        let previous_home = std::env::var("SPLASHBOARD_HOME").ok();
        let previous_filter = std::env::var("SPLASHBOARD_LOG").ok();
        unsafe {
            std::env::set_var("SPLASHBOARD_HOME", &blocked_home);
            std::env::remove_var("SPLASHBOARD_LOG");
        }
        assert!(GUARD.get().is_none());
        init();
        assert!(GUARD.get().is_none());
        unsafe {
            std::env::set_var("SPLASHBOARD_HOME", &good_home);
        }
        init();
        assert!(GUARD.get().is_some());
        tracing::error!("logging smoke test");
        assert!(
            wait_for_log_file(&good_home.join("logs")),
            "log file must be created under {:?}",
            good_home
        );
        unsafe {
            std::env::set_var("SPLASHBOARD_HOME", &later_home);
        }
        init();
        assert!(!later_home.join("logs").exists());
        restore_env("SPLASHBOARD_HOME", previous_home);
        restore_env("SPLASHBOARD_LOG", previous_filter);
        restore_env("SPLASHBOARD_HOME", outer_previous_home);
        restore_env("SPLASHBOARD_LOG", outer_previous_filter);
    }
}
