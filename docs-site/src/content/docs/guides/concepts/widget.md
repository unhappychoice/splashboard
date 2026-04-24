---
title: Widget
description: How a widget is assembled, its lifecycle across cold / warm / animated frames, and the placeholders that substitute when things go sideways.
---

A widget is a single named composition of a fetcher and a renderer,
pinned into a layout slot. The TOML gives it a handle; the runtime
resolves the three pieces, fetches or recomputes the payload, compat-
checks the shape, and paints.

## Anatomy

```toml
[[widget]]
id = "clock"                              # unique handle — rows reference it
fetcher = "clock"                         # who produces the payload
format = "%H:%M"                          # threaded into the fetcher's ctx.format
render = { type = "text_ascii", style = "figlet", font = "banner", align = "center" }
  [widget.options]                        # threaded into the fetcher's ctx.options
  # (clock has none; other fetchers like clock_timezones do)
```

A row then binds the widget into a slot:

```toml
[[row]]
height = { length = 8 }
  [[row.child]]
  widget = "clock"                        # references the id above
```

Widgets and rows live in separate top-level arrays on purpose:

- Widgets are identity + behaviour. A widget has no position; it just
  declares "this is clock_big, it reads from clock and draws with
  text_ascii".
- Rows are composition. A row's child references a widget by id and
  gives it width, border, title, background. Two rows can reference
  the same widget id if they share visual treatment.

## Fields

| field | role |
|---|---|
| `id` | Handle used by rows. Must be unique within the dashboard. |
| `fetcher` | Registered fetcher name (`clock`, `git_status`, `github_repo_prs`, …). |
| `format` | Fetcher-specific format hint. `clock` uses it as a `strftime` string; `basic_static` uses it as a literal body; most others ignore it. |
| `options` | Inline table passed to the fetcher's options deserializer. Each fetcher defines its own typed view via `OptionSchema`. Unknown keys are ignored so adding a new option can't invalidate old configs. |
| `render` | Either a bare string (renderer name) or an inline table `{ type = "…", … }` with renderer-specific options. Omit it to get the shape's default renderer. |

## Shape resolution

Every fetcher declares `shapes()` — the list of shapes it can emit — and
every renderer declares `accepts()` — the list it can draw. The runtime
resolves which shape this widget will use at dispatch time:

1. If `render` pins a renderer, the fetcher's first shape that the
   renderer accepts wins.
2. Otherwise the fetcher's `default_shape()` is used, and the default
   renderer for that shape is picked.
3. If the fetcher emits a body whose shape the renderer doesn't accept,
   the widget renders an in-band `renderer X cannot display Y` error
   rather than crashing the splash.

Multi-shape fetchers (`clock`, `basic_read_store`) branch on
`ctx.shape` inside their `fetch` / `compute`. Single-shape fetchers
ignore it.

## Lifecycle

The same widget takes a different path depending on whether it's cached
or realtime:

```
  Cached widget
  ───────────────────────────────────────────────────────────────────
  new shell                  ┌─ reads ──▶ cache (paint instant)
       │                     │                │
       ▼                     │                │ fresh payload
  splashboard ───────────────┤                ▼
                             │        detached child
                             │                │ runs fetcher with ctx
                             │                ▼
                             └─ writes ──── cache (next render sees fresh)

  Realtime widget  (clock / system_cpu / clock_ratio / pomodoro / …)
  ───────────────────────────────────────────────────────────────────
  draw tick ──▶ compute(ctx) ──▶ payload ──▶ renderer
  (per frame)   < 1 ms, no cache, no I/O
```

The cache lives at `$HOME/.splashboard/cache/<widget_key>.json`. Its
key is `fetcher.cache_key(ctx)` — most fetchers use `name + format`
so two widgets that differ only in `format` get independent cache
slots.

### Loading placeholder

When a cached widget has never run (cache is absent), the runtime
paints a `⏳ …` loading placeholder in the widget's slot and kicks off
the background fetch. The next render picks up the fresh payload.

### Trust placeholder

For local project dashboards (`./.splashboard/dashboard.toml`),
`Network` fetchers are gated behind `splashboard trust`. An untrusted
Network widget renders a `🔒 requires trust` placeholder instead of
running. See [Trust model](/splashboard/guides/trust/).

### Empty-state placeholder

Anything whose body is empty (`TextBlock` with no lines, `Entries`
with no items, `NumberSeries` with no values, …) short-circuits to a
shared "nothing here yet" placeholder *before* renderer dispatch, so
every widget handles "no data" the same way. `Ratio` and `Calendar`
are exceptions — `0% disk used` and "today's month" are both
legitimate.

### Error placeholder

An unknown renderer name, a shape/renderer mismatch, or a fetcher
failure all render an in-band error string. The splash never crashes
the user's prompt.

## Animation window

Most renderers produce the same output every frame, so splashboard
paints once and exits. When a widget's renderer declares
`animates() = true` (`animated_postfx`, `animated_typewriter`,
`animated_figlet_morph`, `animated_boot`, `animated_scanlines`), the
runtime upgrades the draw phase into a multi-frame loop capped at
`ANIMATION_WINDOW` (2 seconds). The final frame is left static, so
the splash rests at the effect's end state rather than mid-motion.

`--wait` blocks until every cached widget has fresh data before
painting the first frame. Without it, the cache-first paint flashes
instantly and fresh data arrives on the next render.

## Config scopes

Which dashboard is in scope depends on where you are:

- **Per-directory** — walk up from CWD looking for
  `./.splashboard/dashboard.toml` or `./.splashboard.toml`. First match
  wins. Travels with a cloned repo.
- **Project fallback** — when CWD is inside a git repo root with no
  per-directory override, `$HOME/.splashboard/project.dashboard.toml`.
- **Home** — everywhere else (not a git repo),
  `$HOME/.splashboard/home.dashboard.toml`.
- **Baked-in defaults** — when none of the above exist, splashboard
  renders its compiled-in fallback so the first run still shows
  something.

Settings (`$HOME/.splashboard/settings.toml`) are global — padding,
theme, viewport height, etc. Dashboards only hold widgets + rows.
