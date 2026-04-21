use std::path::PathBuf;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarsData, Body, CalendarData, EntriesData, HeatmapData, ImageData, LinesData,
    NumberSeriesData, Payload, PointSeriesData, RatioData,
};
use crate::render::Shape;

use super::{FetchContext, FetchError, Fetcher, Safety};

/// Every payload shape ReadStore knows how to deserialize. Intentionally broad — ReadStore is
/// the escape hatch for user-defined widgets, so it supports the non-dynamic shapes exhaustively;
/// config picks which variant a specific file maps to.
const READ_STORE_SHAPES: &[Shape] = &[
    Shape::Lines,
    Shape::Entries,
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::PointSeries,
    Shape::Bars,
    Shape::Image,
    Shape::Calendar,
    Shape::Heatmap,
];

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
    fn shapes(&self) -> &[Shape] {
        READ_STORE_SHAPES
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Differentiate by widget id + declared shape so two widgets pointed at different
        // files (different ids) don't collide, and a single widget changing its shape gets
        // a fresh entry rather than reading an old-shape cached payload.
        let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("");
        let file_format = ctx.file_format.as_deref().unwrap_or("");
        format!(
            "read_store-{}-{}-{}",
            sanitize(&ctx.widget_id),
            shape,
            file_format
        )
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        // Runtime always derives a shape (renderer's single accepted shape, or our
        // `default_shape`); `None` falls back defensively to Lines so direct callers still
        // behave.
        let shape = ctx.shape.unwrap_or(Shape::Lines);
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
fn default_format_for_shape(shape: Shape) -> &'static str {
    match shape {
        Shape::Lines => "text",
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

fn load_body(path: &std::path::Path, file_format: &str, shape: Shape) -> Result<Body, FetchError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty_body(shape)),
        Err(e) => return Err(FetchError::Failed(format!("read failed: {e}"))),
    };
    let text =
        std::str::from_utf8(&bytes).map_err(|e| FetchError::Failed(format!("utf-8: {e}")))?;
    parse_body(text, file_format, shape)
}

fn parse_body(text: &str, file_format: &str, shape: Shape) -> Result<Body, FetchError> {
    match (file_format, shape) {
        ("text", Shape::Lines) => Ok(Body::Lines(LinesData {
            lines: text.lines().map(str::to_string).collect(),
        })),
        ("text", _) => Err(FetchError::Failed(format!(
            "shape {:?} requires json or toml, not text",
            shape.as_str()
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
fn from_json(text: &str, shape: Shape) -> Result<Body, FetchError> {
    macro_rules! parse_as {
        ($ty:ty, $variant:ident) => {
            serde_json::from_str::<$ty>(text)
                .map(Body::$variant)
                .map_err(|e| FetchError::Failed(format!("json parse: {e}")))
        };
    }
    match shape {
        Shape::Lines => parse_as!(LinesData, Lines),
        Shape::Entries => parse_as!(EntriesData, Entries),
        Shape::Ratio => parse_as!(RatioData, Ratio),
        Shape::NumberSeries => parse_as!(NumberSeriesData, NumberSeries),
        Shape::PointSeries => parse_as!(PointSeriesData, PointSeries),
        Shape::Bars => parse_as!(BarsData, Bars),
        Shape::Image => parse_as!(ImageData, Image),
        Shape::Calendar => parse_as!(CalendarData, Calendar),
        Shape::Heatmap => parse_as!(HeatmapData, Heatmap),
        other => Err(FetchError::Failed(format!(
            "read_store doesn't support shape {:?}",
            other.as_str()
        ))),
    }
}

fn from_toml(text: &str, shape: Shape) -> Result<Body, FetchError> {
    macro_rules! parse_as {
        ($ty:ty, $variant:ident) => {
            toml::from_str::<$ty>(text)
                .map(Body::$variant)
                .map_err(|e| FetchError::Failed(format!("toml parse: {e}")))
        };
    }
    match shape {
        Shape::Lines => parse_as!(LinesData, Lines),
        Shape::Entries => parse_as!(EntriesData, Entries),
        Shape::Ratio => parse_as!(RatioData, Ratio),
        Shape::NumberSeries => parse_as!(NumberSeriesData, NumberSeries),
        Shape::PointSeries => parse_as!(PointSeriesData, PointSeries),
        Shape::Bars => parse_as!(BarsData, Bars),
        Shape::Image => parse_as!(ImageData, Image),
        Shape::Calendar => parse_as!(CalendarData, Calendar),
        Shape::Heatmap => parse_as!(HeatmapData, Heatmap),
        other => Err(FetchError::Failed(format!(
            "read_store doesn't support shape {:?}",
            other.as_str()
        ))),
    }
}

/// Empty-but-valid body for the declared shape. Used when the file is missing so the splash
/// stays quiet rather than breaking — matches the "optional" flavor of ReadStore widgets.
fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::Entries => Body::Entries(EntriesData { items: Vec::new() }),
        Shape::Ratio => Body::Ratio(RatioData {
            value: 0.0,
            label: None,
        }),
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData { values: Vec::new() }),
        Shape::PointSeries => Body::PointSeries(PointSeriesData { series: Vec::new() }),
        Shape::Bars => Body::Bars(BarsData {
            bars: Vec::<Bar>::new(),
        }),
        Shape::Image => Body::Image(ImageData {
            path: String::new(),
        }),
        Shape::Calendar => Body::Calendar(CalendarData {
            year: 1970,
            month: 1,
            day: None,
            events: Vec::new(),
        }),
        Shape::Heatmap => Body::Heatmap(HeatmapData {
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

    fn ctx(id: &str, shape: Shape, file_format: &str) -> FetchContext {
        FetchContext {
            widget_id: id.into(),
            timeout: Duration::from_secs(1),
            shape: Some(shape),
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
            .fetch(&ctx("habit", Shape::Heatmap, "json"))
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
            .fetch(&ctx("notes", Shape::Lines, "text"))
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
            .fetch(&ctx("absent", Shape::Heatmap, "json"))
            .await
            .unwrap();
        match p.body {
            Body::Heatmap(d) => assert!(d.cells.is_empty()),
            other => panic!("expected empty heatmap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_shape_falls_back_to_lines() {
        // Runtime always supplies a shape now, but defensively treat `None` as Lines so a stray
        // caller doesn't get an error and the cache_key stays stable.
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("x.txt"), "hello").unwrap();
        let _guard = ScopedHome::new(tmp.path());
        let c = FetchContext {
            widget_id: "x".into(),
            timeout: Duration::from_secs(1),
            file_format: Some("text".into()),
            ..Default::default()
        };
        let p = ReadStoreFetcher.fetch(&c).await.unwrap();
        match p.body {
            Body::Lines(d) => assert_eq!(d.lines, vec!["hello"]),
            other => panic!("expected lines body, got {other:?}"),
        }
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
        let a = ReadStoreFetcher.cache_key(&ctx("habit", Shape::Heatmap, "json"));
        let b = ReadStoreFetcher.cache_key(&ctx("sleep", Shape::Heatmap, "json"));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_differs_across_shapes() {
        let a = ReadStoreFetcher.cache_key(&ctx("habit", Shape::Heatmap, "json"));
        let b = ReadStoreFetcher.cache_key(&ctx("habit", Shape::Lines, "json"));
        assert_ne!(a, b);
    }
}
