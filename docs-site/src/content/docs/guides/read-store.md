---
title: ReadStore
description: Write a payload file, pick a shape, let splashboard draw it — the code-free escape hatch for custom widgets.
---

ReadStore is splashboard's one escape hatch for "I want a widget no
built-in fetcher provides". You write the payload to a file, point a
`basic_read_store` widget at it, and splashboard deserialises the file
into whatever [shape](/guides/concepts/shape/) the paired
renderer expects.

- No code.
- No subprocess.
- Fixed path under `$HOME/.splashboard/store/` — config can't redirect
  the read, so it's
  [Safe](/guides/trust/#safety-classes) even in an
  untrusted local dashboard.

Use it when curated UX (auth, rate limits, stateful update) isn't
warranted — habit trackers, goal progress, custom metrics that cron /
a shell function can emit.

## Quick start

Point a widget at a store id, pair it with a renderer, write the file.

```toml
# $HOME/.splashboard/home.dashboard.toml
[[widget]]
id = "habit"
fetcher = "basic_read_store"
file_format = "json"
render = "gauge_line"
```

```json
// $HOME/.splashboard/store/habit.json
{ "value": 0.6, "label": "habit · 6 of 10 days" }
```

The filename matches the widget `id` (`habit.json`), the shape comes
from the renderer (`gauge_line` accepts `Ratio`), and the file body is
just the inner data — no wrapping envelope.

## File format

### Bare data, no envelope

**This is the one detail that trips everyone up.** ReadStore reads the
file as the inner data struct for the target shape, *not* the
`Body` JSON envelope that splashboard uses internally.

```json
// ✅ ReadStore expects this (the inner RatioData)
{ "value": 0.6, "label": "year progress" }

// ❌ Don't use the Body envelope — ReadStore won't deserialise it
{
  "shape": "ratio",
  "data": { "value": 0.6, "label": "year progress" }
}
```

The envelope form is what you see on the
[Shape page](/guides/concepts/shape/) and inside
splashboard's cache files. ReadStore strips it so your file stays as
short as possible — one concept per file, matching what you'd write if
you'd just invented the format yourself.

### How the shape is picked

The shape isn't in the file — it's picked at runtime from:

1. The renderer's `accepts()` list. `gauge_line` accepts `Ratio`, so a
   `basic_read_store` paired with `gauge_line` always emits `Ratio`.
2. If no renderer is pinned, the fetcher falls back to `TextBlock`
   (safe default). Practically always pin a renderer so ReadStore
   knows what shape to read.

Which means: **the widget's `render` field decides what structure your
file must have.**

### Encodings

`file_format` picks how the file is parsed:

| format | extension | shapes |
|---|---|---|
| `"json"` | `.json` | every structured shape (the default for non-text shapes) |
| `"toml"` | `.toml` | same shapes as JSON |
| `"text"` | `.txt` | `Text` (whole file = one string) and `TextBlock` (one line per row) only |

Omit `file_format` for `Text` / `TextBlock` → defaults to `"text"`.
Omit it for everything else → defaults to `"json"`.

### Missing file

If the file doesn't exist, ReadStore returns an empty body for the
declared shape (empty cells, `0.0` ratio, no entries, …) — the splash
stays quiet rather than erroring. The shared
[empty-state placeholder](/guides/concepts/renderer/#empty-state-handling)
paints in the widget's slot.

## Supported shapes

ReadStore supports every non-dynamic
[shape](/guides/concepts/shape/) — 10 of the 12 variants.
The two exceptions:

- [`Badge`](/guides/concepts/shape/#badge) — status pills
  are always "right now" values; use a built-in fetcher.
- [`Timeline`](/guides/concepts/shape/#timeline) — event
  streams assume dynamic data; ReadStore is for snapshots.

Concrete JSON per supported shape:

### `Text`

```json
// or plain text file with file_format = "text"
{ "value": "deploy ready" }
```

### `TextBlock`

```json
{ "lines": ["feat: foo", "fix: bar", "chore: baz"] }
```

With `file_format = "text"`, one line of the file becomes one entry:

```
feat: foo
fix: bar
chore: baz
```

### `Entries`

```json
{
  "items": [
    { "key": "env", "value": "prod" },
    { "key": "replicas", "value": "3", "status": "ok" },
    { "key": "p99", "value": "420ms", "status": "warn" }
  ]
}
```

`status` is optional (`"ok"` / `"warn"` / `"error"`) and colours the
value in renderers that honour it.

### `Ratio`

```json
{ "value": 0.47, "label": "reading · 14 of 30 books", "denominator": 30 }
```

`label` and `denominator` are both optional.

### `NumberSeries`

```json
{ "values": [2, 3, 1, 5, 8, 13, 4] }
```

### `PointSeries`

```json
{
  "series": [
    { "name": "temp", "points": [[0, 20.0], [1, 21.5], [2, 22.1]] }
  ]
}
```

Multiple series are allowed — each appears as its own line / scatter
cluster depending on the renderer.

### `Bars`

```json
{
  "bars": [
    { "label": "rust", "value": 87000 },
    { "label": "toml", "value": 8000 },
    { "label": "sh",   "value":  500 }
  ]
}
```

### `Image`

```json
{ "path": "/home/you/.cache/me/avatar.png" }
```

Path is absolute — `media_image` opens the file directly.

### `Calendar`

```json
{ "year": 2026, "month": 4, "day": 23, "events": [1, 15, 30] }
```

`day` highlights the focus / "today" cell. `events` is a list of extra
highlighted days (1..=31). Both optional.

### `Heatmap`

```json
{
  "cells": [[0, 1, 3], [2, 5, 1], [0, 0, 4]],
  "thresholds": [1, 3, 5],
  "row_labels": ["Mon", "Tue", "Wed"],
  "col_labels": ["w17", "w18", "w19"]
}
```

Everything except `cells` is optional. Without `thresholds` the
renderer buckets auto-quartiles over the cell values.

## Examples

### Habit tracker

Tick today off, recompute `value = completed / elapsed`, write the
ratio.

```bash
#!/usr/bin/env bash
# ~/.local/bin/habit-tick
store="$HOME/.splashboard/store/habit.json"
log="$HOME/.splashboard/habit.log"
touch "$log"
date +%Y-%m-%d >> "$log"
sort -u -o "$log" "$log"

count=$(grep -c "^$(date +%Y-%m)" "$log")
days=$(date +%d)
value=$(awk "BEGIN { printf \"%.3f\", $count / $days }")

cat > "$store" <<EOF
{ "value": $value, "label": "this month · $count of $days days" }
EOF
```

Paired with:

```toml
[[widget]]
id = "habit"
fetcher = "basic_read_store"
file_format = "json"
render = { type = "gauge_line", label = "habit" }
```

### Goal progress

A yearly target, updated by hand whenever a book lands.

```json
// $HOME/.splashboard/store/reading.json
{ "value": 0.47, "label": "reading · 14 of 30 books" }
```

```toml
[[widget]]
id = "reading"
fetcher = "basic_read_store"
file_format = "json"
render = "gauge_circle"
```

### Custom commit sparkline

Emit a commit-count-per-day number series from a post-commit hook:

```json
// $HOME/.splashboard/store/commits.json
{ "values": [3, 1, 0, 5, 2, 8, 4, 0, 1, 7] }
```

```toml
[[widget]]
id = "commits"
fetcher = "basic_read_store"
file_format = "json"
render = "chart_sparkline"
```

### Workout heatmap

```json
// $HOME/.splashboard/store/workouts.json
{
  "cells": [
    [0, 1, 0, 1, 2, 0, 1],
    [1, 0, 2, 0, 1, 0, 2],
    [2, 1, 0, 1, 0, 2, 1]
  ],
  "row_labels": ["wk17", "wk18", "wk19"],
  "col_labels": ["M", "T", "W", "T", "F", "S", "S"]
}
```

```toml
[[widget]]
id = "workouts"
fetcher = "basic_read_store"
file_format = "json"
render = "grid_heatmap"
```

## Path and safety

- Files live at `$HOME/.splashboard/store/<widget-id>.<ext>`.
- The widget `id` is sanitised — only `[A-Za-z0-9_-]` survive, so a
  hostile repo-local config can't traverse out of the store directory.
- Classified `Safe` in the [trust model](/guides/trust/) —
  always runs, even from an untrusted local dashboard, because the
  read path is fixed.

## What ReadStore isn't

- **Not a subprocess interface.** There's no `command = "..."` or
  plugin protocol; splashboard is a curated renderer, not a shell-script
  host.
- **Not a remote fetcher.** The file path is fixed under `$HOME`. If
  you need a URL, that's a built-in `Network` fetcher territory.
- **Not a full dashboard backend.** If your widget deserves curated
  UX (auth flow, rate limits, stateful update, server-side pagination)
  it lands as a built-in fetcher PR instead.
