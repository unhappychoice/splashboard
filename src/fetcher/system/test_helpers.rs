//! Test fixtures shared across the `system_*` per-fetcher test modules.
//!
//! Compiled only in `cfg(test)` builds. `EnvGuard` serialises env mutation via
//! `TEST_ENV_LOCK` so concurrent tests don't trample each other's `$EDITOR` / `$SHELL` /
//! `$TZ` / ... reads.

#![cfg(test)]

use std::time::Duration;

use crate::paths::TEST_ENV_LOCK;
use crate::render::Shape;

use super::super::FetchContext;
use super::detect_terminal;

pub(crate) struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    restore: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    pub(crate) fn set(pairs: &[(&'static str, Option<&str>)]) -> Self {
        let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut restore: Vec<(&'static str, Option<String>)> = Vec::new();
        for (key, value) in pairs {
            if !restore.iter().any(|(k, _)| k == key) {
                restore.push((*key, std::env::var(key).ok()));
            }
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        Self {
            _lock: lock,
            restore,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        self.restore.iter().for_each(|(key, value)| match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        });
    }
}

pub(crate) const TERMINAL_ENV_KEYS: &[&str] = &[
    "WT_SESSION",
    "GHOSTTY_RESOURCES_DIR",
    "KITTY_WINDOW_ID",
    "TERM",
    "ALACRITTY_WINDOW_ID",
    "ALACRITTY_LOG",
    "WEZTERM_PANE",
    "TERM_PROGRAM",
];

pub(crate) fn ctx_with_shape(shape: Option<Shape>) -> FetchContext {
    FetchContext {
        widget_id: "w".into(),
        timeout: Duration::from_secs(1),
        shape,
        ..Default::default()
    }
}

pub(crate) fn ctx_text(options: Option<&str>) -> FetchContext {
    let options = options.map(|s| toml::from_str::<toml::Value>(s).unwrap());
    FetchContext {
        widget_id: "w".into(),
        timeout: Duration::from_secs(1),
        shape: Some(Shape::Text),
        options,
        ..Default::default()
    }
}

pub(crate) fn detect_terminal_with(overrides: &[(&'static str, &'static str)]) -> String {
    let pairs: Vec<_> = TERMINAL_ENV_KEYS
        .iter()
        .map(|key| {
            (
                *key,
                overrides
                    .iter()
                    .find_map(|(override_key, value)| (*override_key == *key).then_some(*value)),
            )
        })
        .collect();
    let _guard = EnvGuard::set(&pairs);
    detect_terminal()
}
