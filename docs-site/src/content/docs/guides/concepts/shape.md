---
title: Shape
description: The 12 data-shape variants that connect fetchers and renderers, with their payload structure, compatible renderers, and typical fetchers.
---

A **shape** is the structural contract between a fetcher and a
renderer. Neither side knows about the other directly — they both know
shapes. A fetcher declares the shapes it can emit via `shapes()`. A
renderer declares the shapes it can draw via `accepts()`. The runtime
pairs them at dispatch time and surfaces a placeholder if they don't
match.

```
       fetchers emitting Text           renderers accepting Text
       ──────────────────────           ─────────────────────────

           clock ──────┐                 ┌────────── text_plain
           basic_static┤                 ├────────── text_ascii
           git_repo_name┤── Shape::Text ──┤
           ...         ┤                 ├──── animated_typewriter
                       └─────────────────┘

       Any fetcher on the left can drive any renderer on the right.
       Adding one more fetcher that emits Text unlocks all of them.
```

Shapes are the *only* coupling between the two halves. Adding a new
fetcher that emits an existing shape unlocks every compatible renderer
for free; adding a new renderer that accepts an existing shape unlocks
every fetcher emitting it.

## Overview

| shape | default renderer | also accepted by | used for |
|---|---|---|---|
| [`Text`](#text) | `text_plain` | `text_ascii`, `animated_typewriter`, `animated_postfx` | single-string values |
| [`TextBlock`](#textblock) | `text_plain` | `list_plain` | zero or more lines |
| [`Entries`](#entries) | `grid_table` | — | key / value rows |
| [`Ratio`](#ratio) | `gauge_circle` | `gauge_line` | a `0..=1` progress |
| [`NumberSeries`](#numberseries) | `chart_sparkline` | `chart_bar` | sequence of integers |
| [`PointSeries`](#pointseries) | `chart_line` | `chart_scatter` | (x, y) samples |
| [`Bars`](#bars) | `chart_bar` | `chart_pie` | labeled magnitudes |
| [`Image`](#image) | `media_image` | — | PNG / JPEG path |
| [`Calendar`](#calendar) | `grid_calendar` | — | month view |
| [`Heatmap`](#heatmap) | `grid_heatmap` | — | 2D intensity grid |
| [`Badge`](#badge) | `status_badge` | — | traffic-light status |
| [`Timeline`](#timeline) | `list_timeline` | — | chronological events |

The authoritative compatibility matrix is in the
[reference overview](/splashboard/reference/matrix/) — generated from
each renderer's `accepts()`, so it can't drift.

## `Text`

A single string. The workhorse — anything that reduces to one line.

```rust
Body::Text(TextData { value: String })
```

```json
{ "shape": "text", "data": { "value": "14:32" } }
```

Renderers: `text_plain` (default), `text_ascii` (figlet / block letters),
`animated_typewriter`, `animated_postfx`.

Typical fetchers: `clock`, `clock_derived`, `git_repo_name`, `git_status`,
`github_repo_stars`, `basic_static`.

## `TextBlock`

Zero or more lines of text.

```rust
Body::TextBlock(TextBlockData { lines: Vec<String> })
```

```json
{ "shape": "text_block", "data": { "lines": ["feat: add foo", "fix: bar"] } }
```

Renderers: `text_plain` (default), `list_plain`.

Typical fetchers: `git_recent_commits`, `github_user`, `github_repo`,
`basic_static` (when format contains newlines).

## `Entries`

Key/value rows with optional per-row status (colours the value).

```rust
Body::Entries(EntriesData { items: Vec<Entry { key, value, status }> })
```

```json
{
  "shape": "entries",
  "data": { "items": [
    { "key": "os", "value": "macOS 14.3" },
    { "key": "load", "value": "1.24", "status": "ok" },
    { "key": "disk", "value": "91%", "status": "warn" }
  ]}
}
```

Renderers: `grid_table` (default).

Typical fetchers: `system`, `clock_almanac`, `clock_timezones`,
`github_repo_stars`.

## `Ratio`

A single `0.0..=1.0` progress value. Never considered empty (`0%` is
legitimate).

```rust
Body::Ratio(RatioData { value: f64, label: Option<String>, denominator: Option<u64> })
```

```json
{ "shape": "ratio", "data": { "value": 0.32, "label": "year", "denominator": 365 } }
```

Renderers: `gauge_circle` (default), `gauge_line`.

Typical fetchers: `clock_ratio` (year / quarter / month / week / day
progress), `system_cpu`, `system_memory`, `system_disk_usage`.

## `NumberSeries`

A sequence of unsigned integers. Good for anything "counts over time".

```rust
Body::NumberSeries(NumberSeriesData { values: Vec<u64> })
```

```json
{ "shape": "number_series", "data": { "values": [2, 3, 1, 5, 8, 13, 4] } }
```

Renderers: `chart_sparkline` (default), `chart_bar`.

Typical fetchers: `git_commits_activity` (when rendered as a sparkline),
`basic_read_store`.

## `PointSeries`

One or more named series of `(x, y)` samples. Good for continuous data.

```rust
Body::PointSeries(PointSeriesData {
    series: Vec<PointSeries { name: String, points: Vec<(f64, f64)> }>,
})
```

```json
{
  "shape": "point_series",
  "data": { "series": [
    { "name": "temp", "points": [[0, 20.0], [1, 21.5], [2, 22.1]] }
  ]}
}
```

Renderers: `chart_line` (default), `chart_scatter`.

Typical fetchers: `basic_read_store` (any custom metric with an X axis).

## `Bars`

Labeled, non-negative magnitudes.

```rust
Body::Bars(BarsData { bars: Vec<Bar { label: String, value: u64 }> })
```

```json
{
  "shape": "bars",
  "data": { "bars": [
    { "label": "Rust", "value": 87000 },
    { "label": "TOML", "value": 8000 }
  ]}
}
```

Renderers: `chart_bar` (default), `chart_pie`.

Typical fetchers: `git_contributors`, `github_languages`.

## `Image`

Path to an image on disk. Terminal renders it via
[viuer](https://github.com/atanunq/viuer) (kitty / iTerm2 / sixel
protocols where supported, falling back to unicode blocks).

```rust
Body::Image(ImageData { path: String })
```

```json
{ "shape": "image", "data": { "path": "/home/you/.cache/splashboard/avatar.png" } }
```

Renderers: `media_image` (default).

Typical fetchers: `github_avatar`, `basic_read_store`.

## `Calendar`

A month view. Never considered empty (the anchor month is always
present).

```rust
Body::Calendar(CalendarData {
    year: i32,
    month: u8,        // 1..=12
    day: Option<u8>,  // today / focus highlight
    events: Vec<u8>,  // extra highlighted days (1..=31)
})
```

```json
{ "shape": "calendar", "data": { "year": 2026, "month": 4, "day": 23, "events": [1, 15, 30] } }
```

Renderers: `grid_calendar` (default).

Typical fetchers: `clock` (current month).

## `Heatmap`

2D intensity grid, row-major. The renderer buckets cells via explicit
`thresholds` or auto-quartiles; edge labels render when there's room.

```rust
Body::Heatmap(HeatmapData {
    cells: Vec<Vec<u32>>,
    thresholds: Option<Vec<u32>>,
    row_labels: Option<Vec<String>>,
    col_labels: Option<Vec<String>>,
})
```

```json
{
  "shape": "heatmap",
  "data": {
    "cells": [[0, 1, 3], [2, 5, 1], [0, 0, 4]],
    "thresholds": [1, 3, 5]
  }
}
```

Renderers: `grid_heatmap` (default).

Typical fetchers: `git_commits_activity`, `github_contributions`,
`git_blame_heatmap`.

## `Badge`

A single traffic-light status with a short label. One indicator per
widget; row-of-badges is a layout concern, not a shape.

```rust
Body::Badge(BadgeData { status: Status, label: String })
// Status = Ok | Warn | Error
```

```json
{ "shape": "badge", "data": { "status": "warn", "label": "deploy degraded" } }
```

Renderers: `status_badge` (default), `grid_table` (as a one-row table).

Typical fetchers: — (no built-in Badge emitter today; user-written
ReadStore widgets are the main consumer).

## `Timeline`

Time-stamped events, newest first. Timestamps are raw unix seconds
UTC; the renderer formats `"3h ago"` / `"yesterday"` at draw time so
cached payloads don't freeze stale relative labels.

```rust
Body::Timeline(TimelineData {
    events: Vec<TimelineEvent { timestamp: i64, title: String, detail: Option<String>, status: Option<Status> }>,
})
```

```json
{
  "shape": "timeline",
  "data": { "events": [
    { "timestamp": 1700000000, "title": "merged #42", "detail": "feat: heatmap", "status": "ok" },
    { "timestamp": 1699990000, "title": "opened #41" }
  ]}
}
```

Renderers: `list_timeline` (default).

Typical fetchers: `github_notifications`, `github_recent_releases`,
`git_recent_commits` (when rendered as a timeline).

## Empty-state rules

Most shapes have a natural "empty" state that the runtime detects
before renderer dispatch, substituting the shared "nothing here yet"
placeholder:

- `Text` — `value.is_empty()`
- `TextBlock` — no lines or all lines blank
- `Entries`, `NumberSeries`, `PointSeries`, `Bars`, `Timeline` — empty
  collection
- `Image` — empty path
- `Heatmap` — no cells
- `Badge` — empty label

`Ratio` and `Calendar` are never considered empty — `0%` and "this
month" are both legitimate values.

## Adding a new shape

Most widgets can be expressed with an existing shape. Add a new one
only when none of the 12 carries the structural information the
renderer needs. The process:

1. Add a new `Body::XXX(XxxData)` variant in `src/payload.rs` with a
   serde-serializable struct.
2. Add a matching `Shape::XXX` variant in `src/render/mod.rs` and
   handle it in `shape_of()`.
3. Add an entry in `default_renderer_for()`.
4. Handle the new variant in `is_empty_body()`.
5. Implement at least one renderer that lists the new shape in its
   `accepts()`.

Shapes are a hard coupling — adding one obliges every touching side to
know about it. "This renderer needs the raw fetcher data" is almost
never a reason to add a shape; it's a reason to add a new renderer
that draws an existing shape differently.
