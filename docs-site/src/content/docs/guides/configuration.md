---
title: Configuration
description: The TOML schema for settings, dashboards, widgets, rows, and render options.
---

This page is the TOML schema. If you haven't read
[Concepts](/guides/concepts/) yet, start there — every field
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
├── secrets.toml               # tokens / env vars exported at startup
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

# Auto-display toggles — both default true. Set independently to mute one
# trigger without unwiring the shell rc. See the cookbook for use cases.
auto_home  = true   # render the home dashboard at shell startup
auto_on_cd = true   # repaint when entering a project directory

[theme]
# Base palette — one of "default", "tokyo_night", "nord", "dracula",
# "gruvbox_dark", "catppuccin_mocha". See the themes guide.
preset = "tokyo_night"

# Per-token overrides. Any key from the theme schema is accepted; unknown
# keys fall back to the preset's value.
panel_title = "#ff0088"
```

Full theme token list and the `"reset"` escape hatch for light terminals:
see [Themes](/guides/themes/).

## `secrets.toml`

Tokens and other env vars that fetchers need (`GH_TOKEN`, `GITHUB_TOKEN`,
`TODOIST_TOKEN`, …) can live in a flat top-level TOML map at
`$HOME/.splashboard/secrets.toml`. splashboard reads it once at startup and
exports each entry as a process env var, so any fetcher that looks up the
matching env name picks it up without touching your shell rc.

```toml
# $HOME/.splashboard/secrets.toml
GH_TOKEN      = "ghp_..."
TODOIST_TOKEN = "..."
```

Rules:

- **Shell env always wins.** A key already defined in the environment is
  left alone, so an ad-hoc `GH_TOKEN=... splashboard ...` override still
  works.
- **Filtered keys.** `SPLASHBOARD_*` (splashboard's own knobs) and ambient
  process state owned by the shell — `PATH`, `HOME`, `SHELL`, `USER`,
  `LOGNAME`, `PWD`, `OLDPWD`, `IFS`, `LD_PRELOAD`, `LD_LIBRARY_PATH`,
  `DYLD_INSERT_LIBRARIES`, `DYLD_LIBRARY_PATH` — are silently ignored, so a
  stray entry can't redirect ambient state.
- **Best-effort.** A missing file is fine; a parse error logs a warning and
  the splash still renders.
- **Git-ignored by `splashboard install`.** The installer drops
  `secrets.toml` into a sibling `$HOME/.splashboard/.gitignore` (idempotent),
  so dotfiles-in-git users who commit the rest of `.splashboard/` don't leak
  their tokens by accident.

Lives in its own file (not `settings.toml`) so dotfiles-in-git users can
git-ignore exactly the file that holds tokens without losing the rest of
their splashboard config. See the
[cookbook](/guides/cookbook/#github-credentials) for example wiring.

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
format = "%H:%M"                              # clock / basic_static-style option
```

Fields:

- **`id`** (required) — unique string used to reference the widget from
  rows.
- **`fetcher`** (required) — registry name. Browse with
  [`splashboard catalog fetcher`](/reference/matrix/).
- **`render`** — renderer selection. Short form `render = "text_plain"` or
  long form `render = { type = "text_ascii", pixel_size = "quadrant" }`.
  Absent = pick the default renderer for the fetcher's shape.
- **`format`** — fetcher-specific format string where applicable (e.g.
  `clock` accepts strftime patterns, `basic_static` takes literal text).
- **`refresh_interval`** — cache TTL for cached fetchers, in seconds.
  Ignored by realtime fetchers.
- **`file_format`** — `basic_read_store` only: `"json"`, `"toml"`, or
  `"text"`. The file is deserialized into the
  [shape](/guides/concepts/shape/) the paired renderer
  expects.
- **`[widget.options]`** — fetcher-specific nested options. See each
  fetcher's reference page for its schema.

### `[[row]]` and `[[row.child]]`

Rows stack vertically. Each row's children stack horizontally.

```toml
[[row]]
height = { length = 6 }     # cells — or { fill = 1 }, { min = 3 }, { percentage = 50 }
bg = "subtle"               # "default" or "subtle" (paints theme.bg_subtle)
flex = "center"             # how children distribute when they don't fill the row
gap = 2                     # cells of blank space between sibling children
title = "system"            # optional row-level title (goes with a row border)
border = "top"              # "none" | "plain" | "rounded" | "thick" | "double" | "top"

  [[row.child]]
  widget = "clock"          # widget id from [[widget]] — omit for a pure layout spacer
  width = { length = 30 }   # same size shapes as row height
  title = "now"             # optional child-level title
  border = "rounded"        # optional child-level chrome
  bg = "subtle"             # optional per-child bg
```

Two ways to carve whitespace between children: `gap = N` on the row (sugar
for the common case) auto-splices `N`-cell spacers between every pair; or
drop a `[[row.child]]` with `width = { length = N }` and **no `widget =`**
for an explicit spacer you can place anywhere in the row.

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

See each renderer's [reference page](/reference/matrix/) for the
definitive list.

### A worked example

```toml
# ./.splashboard/dashboard.toml — a three-row project splash

[[widget]]
id = "hero"
fetcher = "git_repo_name"
render = { type = "text_ascii", style = "figlet", font = "banner", align = "center", color = "panel_title" }

[[widget]]
id = "subtitle"
fetcher = "github_repo"
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
