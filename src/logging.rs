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
    use std::time::Duration;
    use tracing_appender::non_blocking;
    use tracing_subscriber::fmt::MakeWriter;

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
        // The appender rotates daily; the current day's file starts with our prefix.
        let mut found = false;
        for _ in 0..40 {
            let any = std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .any(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(LOG_FILENAME_PREFIX)
                });
            if any {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(found, "log file must be created under {:?}", dir.path());
    }
}
