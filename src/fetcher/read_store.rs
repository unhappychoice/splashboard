use std::path::PathBuf;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarsData, Body, CalendarData, EntriesData, HeatmapData, ImageData, LinesData,
    NumberSeriesData, Payload, PointSeriesData, RatioData,
};

use super::{FetchContext, FetchError, Fetcher, Safety};

/// Renders a local file the user wrote. The file lives at a fixed, sanitized path
/// `$HOME/.splashboard/store/<widget_id>.<ext>` — config cannot redirect the read, so a hostile
/// repo-local config can't traverse the filesystem. The value on disk is user-controlled;
/// splashboard just deserializes it into a `Payload` of the shape the config declares.
///
/// Fills the gap that #5 (plugin protocol) and #20 (command widget) were meant to cover:
/// arbitrary user-defined display surface, without any exec path. Users populate the file via
/// whatever they like (editor, cron, post-commit hook, CI step).
pub struct ReadStoreFetcher;

#[async_trait]
impl Fetcher for ReadStoreFetcher {
    fn name(&self) -> &str {
        "read_store"
    }
    fn safety(&self) -> Safety {
        // Always Safe: the path is derived from the widget id under a fixed home subdir, so
        // there's no path-traversal vector even in an untrusted local config.
        Safety::Safe
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Differentiate by widget id + declared shape so two widgets pointed at different
        // files (different ids) don't collide, and a single widget changing its shape gets
        // a fresh entry rather than reading an old-shape cached payload.
        let shape = ctx.shape.as_deref().unwrap_or("");
        let file_format = ctx.file_format.as_deref().unwrap_or("");
        format!(
            "read_store-{}-{}-{}",
            sanitize(&ctx.widget_id),
            shape,
            file_format
        )
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let shape = ctx
            .shape
            .as_deref()
            .ok_or_else(|| FetchError::Failed("read_store widget missing `shape`".into()))?;
        let file_format = ctx
            .file_format
            .as_deref()
            .unwrap_or(default_format_for_shape(shape));
        let path = resolve_path(&ctx.widget_id, file_format)
            .ok_or_else(|| FetchError::Failed("could not resolve splashboard home".into()))?;
        let body = load_body(&path, file_format, shape)?;
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body,
        })
    }
}

/// `text` is the natural default for `lines`; everything else needs structure. Saves users
/// from writing `file_format = "text"` for every simple notes widget.
fn default_format_for_shape(shape: &str) -> &'static str {
    match shape {
        "lines" => "text",
        _ => "json",
    }
}

fn resolve_path(widget_id: &str, file_format: &str) -> Option<PathBuf> {
    let ext = extension_for(file_format);
    let name = format!("{}.{ext}", sanitize(widget_id));
    crate::paths::read_store_dir().map(|d| d.join(name))
}

fn extension_for(file_format: &str) -> &'static str {
    match file_format {
        "toml" => "toml",
        "text" => "txt",
        _ => "json",
    }
}

/// Confine filenames to safe characters so widget ids never escape the store subdir or break
/// on case-insensitive filesystems. Same rule as the cache module.
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

fn load_body(path: &std::path::Path, file_format: &str, shape: &str) -> Result<Body, FetchError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty_body(shape)),
        Err(e) => return Err(FetchError::Failed(format!("read failed: {e}"))),
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| FetchError::Failed(format!("utf-8: {e}")))?;
    parse_body(text, file_format, shape)
}

fn parse_body(text: &str, file_format: &str, shape: &str) -> Result<Body, FetchError> {
    match (file_format, shape) {
        ("text", "lines") => Ok(Body::Lines(LinesData {
            lines: text.lines().map(str::to_string).collect(),
        })),
        ("text", _) => Err(FetchError::Failed(format!(
            "shape {shape:?} requires json or toml, not text"
        ))),
        ("json", shape) => from_json(text, shape),
        ("toml", shape) => from_toml(text, shape),
        (other, _) => Err(FetchError::Failed(format!(
            "unknown file_format: {other:?}"
        ))),
    }
}

/// Deserialize the file as the inner `*Data` struct for the requested shape, and wrap in the
/// right `Body` variant. Users write the data payload directly (e.g. `{ "cells": [[...]] }`
/// for a heatmap) — no wrapping `{"shape":...}` envelope required.
fn from_json(text: &str, shape: &str) -> Result<Body, FetchError> {
    macro_rules! parse_as {
        ($ty:ty, $variant:ident) => {
            serde_json::from_str::<$ty>(text)
                .map(Body::$variant)
                .map_err(|e| FetchError::Failed(format!("json parse: {e}")))
        };
    }
    match shape {
        "lines" => parse_as!(LinesData, Lines),
        "entries" => parse_as!(EntriesData, Entries),
        "ratio" => parse_as!(RatioData, Ratio),
        "number_series" => parse_as!(NumberSeriesData, NumberSeries),
        "point_series" => parse_as!(PointSeriesData, PointSeries),
        "bars" => parse_as!(BarsData, Bars),
        "image" => parse_as!(ImageData, Image),
        "calendar" => parse_as!(CalendarData, Calendar),
        "heatmap" => parse_as!(HeatmapData, Heatmap),
        other => Err(FetchError::Failed(format!("unknown shape: {other:?}"))),
    }
}

fn from_toml(text: &str, shape: &str) -> Result<Body, FetchError> {
    macro_rules! parse_as {
        ($ty:ty, $variant:ident) => {
            toml::from_str::<$ty>(text)
                .map(Body::$variant)
                .map_err(|e| FetchError::Failed(format!("toml parse: {e}")))
        };
    }
    match shape {
        "lines" => parse_as!(LinesData, Lines),
        "entries" => parse_as!(EntriesData, Entries),
        "ratio" => parse_as!(RatioData, Ratio),
        "number_series" => parse_as!(NumberSeriesData, NumberSeries),
        "point_series" => parse_as!(PointSeriesData, PointSeries),
        "bars" => parse_as!(BarsData, Bars),
        "image" => parse_as!(ImageData, Image),
        "calendar" => parse_as!(CalendarData, Calendar),
        "heatmap" => parse_as!(HeatmapData, Heatmap),
        other => Err(FetchError::Failed(format!("unknown shape: {other:?}"))),
    }
}

/// Empty-but-valid body for the declared shape. Used when the file is missing so the splash
/// stays quiet rather than breaking — matches the "optional" flavor of ReadStore widgets.
fn empty_body(shape: &str) -> Body {
    match shape {
        "entries" => Body::Entries(EntriesData { items: Vec::new() }),
        "ratio" => Body::Ratio(RatioData {
            value: 0.0,
            label: None,
        }),
        "number_series" => Body::NumberSeries(NumberSeriesData { values: Vec::new() }),
        "point_series" => Body::PointSeries(PointSeriesData { series: Vec::new() }),
        "bars" => Body::Bars(BarsData { bars: Vec::<Bar>::new() }),
        "image" => Body::Image(ImageData { path: String::new() }),
        "calendar" => Body::Calendar(CalendarData {
            year: 1970,
            month: 1,
            day: None,
            events: Vec::new(),
        }),
        "heatmap" => Body::Heatmap(HeatmapData {
            cells: Vec::new(),
            thresholds: None,
            row_labels: None,
            col_labels: None,
        }),
        _ => Body::Lines(LinesData { lines: Vec::new() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ctx(id: &str, shape: &str, file_format: &str) -> FetchContext {
        FetchContext {
            widget_id: id.into(),
            timeout: Duration::from_secs(1),
            shape: Some(shape.into()),
            file_format: Some(file_format.into()),
            ..Default::default()
        }
    }

    /// Scope-guard sharing the crate-wide `paths::TEST_ENV_LOCK`, so any parallel test that
    /// also touches `SPLASHBOARD_HOME` serializes with us.
    struct ScopedHome {
        previous: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl ScopedHome {
        fn new(dir: &std::path::Path) -> Self {
            let lock = crate::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var("SPLASHBOARD_HOME").ok();
            unsafe {
                std::env::set_var("SPLASHBOARD_HOME", dir);
            }
            Self {
                previous,
                _lock: lock,
            }
        }
    }
    impl Drop for ScopedHome {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(v) => std::env::set_var("SPLASHBOARD_HOME", v),
                    None => std::env::remove_var("SPLASHBOARD_HOME"),
                }
            }
        }
    }

    #[tokio::test]
    async fn reads_heatmap_json() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("habit.json"), r#"{"cells":[[0,1,2,3,4]]}"#).unwrap();
        let _guard = ScopedHome::new(tmp.path());
        let p = ReadStoreFetcher
            .fetch(&ctx("habit", "heatmap", "json"))
            .await
            .unwrap();
        match p.body {
            Body::Heatmap(d) => assert_eq!(d.cells, vec![vec![0, 1, 2, 3, 4]]),
            other => panic!("expected heatmap body, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn text_shape_lines_splits_on_newline() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("notes.txt"), "one\ntwo\nthree").unwrap();
        let _guard = ScopedHome::new(tmp.path());
        let p = ReadStoreFetcher
            .fetch(&ctx("notes", "lines", "text"))
            .await
            .unwrap();
        match p.body {
            Body::Lines(d) => assert_eq!(d.lines, vec!["one", "two", "three"]),
            other => panic!("expected lines body, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_file_renders_empty_for_declared_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = ScopedHome::new(tmp.path());
        let p = ReadStoreFetcher
            .fetch(&ctx("absent", "heatmap", "json"))
            .await
            .unwrap();
        match p.body {
            Body::Heatmap(d) => assert!(d.cells.is_empty()),
            other => panic!("expected empty heatmap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_shape_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("x.json"), "{}").unwrap();
        let _guard = ScopedHome::new(tmp.path());
        let p = ReadStoreFetcher
            .fetch(&ctx("x", "definitely_not_a_shape", "json"))
            .await;
        assert!(matches!(p, Err(FetchError::Failed(_))));
    }

    #[tokio::test]
    async fn missing_shape_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let _guard = ScopedHome::new(tmp.path());
        let c = FetchContext {
            widget_id: "x".into(),
            timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let p = ReadStoreFetcher.fetch(&c).await;
        assert!(matches!(p, Err(FetchError::Failed(_))));
    }

    #[test]
    fn sanitize_drops_path_traversal() {
        assert_eq!(sanitize("../etc/passwd"), "___etc_passwd");
        assert_eq!(sanitize("../../secret"), "______secret");
        assert_eq!(sanitize("a b/c"), "a_b_c");
        assert_eq!(sanitize("habit-01_alt"), "habit-01_alt");
    }

    #[test]
    fn cache_key_differs_across_widget_ids() {
        let a = ReadStoreFetcher.cache_key(&ctx("habit", "heatmap", "json"));
        let b = ReadStoreFetcher.cache_key(&ctx("sleep", "heatmap", "json"));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_differs_across_shapes() {
        let a = ReadStoreFetcher.cache_key(&ctx("habit", "heatmap", "json"));
        let b = ReadStoreFetcher.cache_key(&ctx("habit", "lines", "json"));
        assert_ne!(a, b);
    }
}
