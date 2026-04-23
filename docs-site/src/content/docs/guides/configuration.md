---
title: Configuration
description: The TOML schema for settings, dashboards, widgets, rows, and render options.
---

This page is the TOML schema. If you haven't read
[Concepts](/splashboard/guides/concepts/) yet, start there — every field
below is easier to understand once the Widget = Fetcher + Renderer +
Layout slot equation has clicked.

splashboard splits configuration into two files by design:

- **settings** — user preferences that apply to every dashboard (theme,
  padding, inline viewport height). Lives in HOME only.
- **dashboard** — what widgets to show and where. Resolved per-context:
  per-directory, project, or home.

Keeping them separate means you set your theme once and every splash follows,
while each context (home vs. a specific project) can swap its widget layout
independently.

## File locations

All splashboard state lives under `$HOME/.splashboard/` (same on Linux,
macOS, and Windows — no XDG paths). Override with `SPLASHBOARD_HOME` for
tests or relocatable installs.

```
$HOME/.splashboard/
├── settings.toml              # [general], [theme]
├── home.dashboard.toml        # ambient splash
├── project.dashboard.toml     # repo-root splash when the repo ships no config
├── trust.toml                 # trust store (auto-managed)
├── cache/                     # per-widget cache
└── store/                     # ReadStore files
```

Per-directory dashboards stay in the repo as either:

- `./.splashboard/dashboard.toml`
- `./.splashboard.toml`

## How a splash resolves

splashboard picks the dashboard at render time based on the current
directory:

```
                   splashboard render
                          │
                          ▼
   ┌───────────────────────────────────────────┐
   │ CWD has ./.splashboard/dashboard.toml     │
   │ or ./.splashboard.toml ?                  │─── yes ─▶  per-dir
   └───────────────────┬───────────────────────┘           (trust-gated)
                       │ no
                       ▼
   ┌───────────────────────────────────────────┐
   │ CWD is a git repo root (has .git/) ?      │─── yes ─▶  $HOME/.splashboard/
   └───────────────────┬───────────────────────┘          project.dashboard.toml
                       │ no
                       ▼
                $HOME/.splashboard/
                home.dashboard.toml
```

1. **Per-dir dashboard at CWD** (`./.splashboard/dashboard.toml` or
   `./.splashboard.toml`) — if present, it wins. Trust-gated: Network
   widgets are replaced with a placeholder until the file is trusted.
2. **Project dashboard** — when CWD is a git repo root (i.e. contains
   `.git`) and no per-dir dashboard applies. Loads
   `$HOME/.splashboard/project.dashboard.toml`.
3. **Home dashboard** — everything else. Loads
   `$HOME/.splashboard/home.dashboard.toml`.

Per-dir dashboards only activate at the **directory itself**, not walking up
the tree. Subdirectories inside a repo fall through to the home dashboard, so
`cd` into `src/foo/` doesn't repaint the project splash over and over.

## `settings.toml`

```toml
[general]
# Block the initial paint until every widget has fresh data. Default false
# (paint cached immediately, refresh in the background).
wait_for_fresh = false

# Inline viewport height in rows. Omit to use the computed default.
height = 24

# Space reserved around the root layout. Either `padding = 1` (uniform) or
# `{ x = 2, y = 1 }` (split horizontal / vertical).
padding = { x = 2, y = 1 }

[theme]
# Base palette — one of "default", "tokyo_night", "nord", "dracula",
# "gruvbox_dark", "catppuccin_mocha". See the themes guide.
preset = "tokyo_night"

# Per-token overrides. Any key from the theme schema is accepted; unknown
# keys fall back to the preset's value.
panel_title = "#ff0088"
```

Full theme token list and the `"reset"` escape hatch for light terminals:
see [Themes](/splashboard/guides/themes/).

## Dashboard files

A dashboard file holds **widgets** (what to show) and **rows** (where to put
them). Settings keys like `[general]` or `[theme]` do not belong here — they
go in `settings.toml`.

### `[[widget]]`

```toml
[[widget]]
id = "clock"                                  # unique within the dashboard
fetcher = "clock"                             # registry name
render = { type = "text_ascii", align = "center" }
refresh_interval = 30                         # cached fetcher TTL seconds
format = "%H:%M"                              # clock / static-style option
```

Fields:

- **`id`** (required) — unique string used to reference the widget from
  rows.
- **`fetcher`** (required) — registry name. Browse with
  [`splashboard catalog fetcher`](/splashboard/reference/matrix/).
- **`render`** — renderer selection. Short form `render = "text_plain"` or
  long form `render = { type = "text_ascii", pixel_size = "quadrant" }`.
  Absent = pick the default renderer for the fetcher's shape.
- **`format`** — fetcher-specific format string where applicable (e.g.
  `clock` accepts strftime patterns, `static` takes literal text).
- **`refresh_interval`** — cache TTL for cached fetchers, in seconds.
  Ignored by realtime fetchers.
- **`file_format`** — `read_store` only: `"json"`, `"toml"`, or `"text"`.
- **`[widget.options]`** — fetcher-specific nested options. See each
  fetcher's reference page for its schema.

### `[[row]]` and `[[row.child]]`

Rows stack vertically. Each row's children stack horizontally.

```toml
[[row]]
height = { length = 6 }     # cells — or { fill = 1 }, { min = 3 }, { percentage = 50 }
bg = "subtle"               # "default" or "subtle" (paints theme.bg_subtle)
flex = "center"             # how children distribute when they don't fill the row
title = "system"            # optional row-level title (goes with a row border)
border = "top"              # "none" | "plain" | "rounded" | "thick" | "double" | "top"

  [[row.child]]
  widget = "clock"          # widget id from [[widget]]
  width = { length = 30 }   # same size shapes as row height
  title = "now"             # optional child-level title
  border = "rounded"        # optional child-level chrome
  bg = "subtle"             # optional per-child bg
```

Size forms (`height`, `width`):

- `{ length = N }` — fixed N cells.
- `{ fill = N }` — proportional to siblings with `fill`.
- `{ min = N }` — at least N cells, grows if slack is available.
- `{ max = N }` — at most N cells.
- `{ percentage = N }` — N% of the parent's axis.

Border shorthand: `border = "top"` + `title = "..."` on a row paints a
section divider — a top rule with the title hanging off it, no side or
bottom chrome.

`flex = "center"` is the idiomatic way to park narrower children in the
middle of a wider row (useful for subtitle lines or single-column widgets
that shouldn't stretch).

## Render options

Renderers share a flat options bag; each renderer picks the fields it cares
about, others are ignored. The common ones:

- **`align`** (`"left"` / `"center"` / `"right"`) — honoured by `text_plain`,
  `text_ascii`, and `grid_heatmap`. Structural renderers (`grid_table`,
  `gauge_*`, `chart_*`) ignore it.
- **`color`** — theme token name overriding the renderer's default
  foreground (e.g. `"panel_title"`, `"status_ok"`). Honoured by `text_ascii`
  and `status_badge`.
- **`style`** — visual variant: `text_ascii` takes `"blocks"` / `"figlet"`;
  `chart_sparkline` takes `"bars"` / `"line"`.
- **`font`** — `text_ascii` figlet font: `"standard"`, `"small"`, `"big"`,
  `"banner"`. Only applies when `style = "figlet"`.
- **`pixel_size`** — `text_ascii` blocks style: `"full"`, `"quadrant"`, or
  `"sextant"`. Omit to let the renderer pick based on the area height.
- **`max_items`** — list renderers: cap on items rendered.
- **`bullet`** — list renderers: leading glyph per item.
- **`bar_width`**, **`horizontal`**, **`stacked`**, **`max_bars`** —
  `chart_bar` options.
- **`ring_thickness`**, **`label_position`** — `gauge_circle` options.

See each renderer's [reference page](/splashboard/reference/matrix/) for the
definitive list.

### A worked example

```toml
# ./.splashboard/dashboard.toml — a three-row project splash

[[widget]]
id = "hero"
fetcher = "project_name"
render = { type = "text_ascii", style = "figlet", font = "banner", align = "center", color = "panel_title" }

[[widget]]
id = "subtitle"
fetcher = "project"
render = { type = "text_plain", align = "center" }

[[widget]]
id = "commits"
fetcher = "git_commits_activity"
render = "grid_heatmap"

[[widget]]
id = "releases"
fetcher = "github_recent_releases"
render = { type = "list_timeline", bullet = "●", max_items = 3 }

[[row]]
height = { length = 8 }
bg = "subtle"
  [[row.child]]
  widget = "hero"

[[row]]
height = { length = 3 }
bg = "subtle"
flex = "center"
  [[row.child]]
  widget = "subtitle"
  width = { length = 70 }

[[row]]
height = { length = 1 }

[[row]]
height = { length = 10 }
flex = "center"
  [[row.child]]
  widget = "commits"
  width = { length = 34 }
  border = "top"
  title = "commits"
  [[row.child]]
  widget = "releases"
  width = { length = 34 }
  border = "top"
  title = "released"
```

The header band paints on `bg.subtle` to lift it off the main content. Rows
beneath split into two equal columns with a top rule and title over each.
