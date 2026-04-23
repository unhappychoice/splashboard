---
title: Concepts
description: The mental model — Widget = Fetcher + Renderer + Layout slot.
---

Once you internalise one equation, every other page clicks into place:

```
Widget = Fetcher × Renderer × Layout slot
```

A **widget** is not a single built-in thing. It's a composition of three
independent pieces that splashboard combines at render time. Splitting those
axes is the whole point: the same data can drive multiple visuals, the same
visual can consume multiple data sources, and each piece evolves on its own.

```
          one Widget = one composition of three pieces

    Fetcher                Renderer               Layout slot
 ┌────────────┐       ┌──────────────┐      ┌──────────────┐
 │   clock    │──────▶│  text_ascii  │─────▶│ row, height 4│
 │ emits Text │       │ draws blocks │      │   centered   │
 └────────────┘       └──────────────┘      └──────────────┘
      WHAT                  HOW                   WHERE
```

## The three pieces

### Fetcher — what data

A fetcher produces a `Payload`. That's it. A `clock` fetcher emits the
current time. A `git_status` fetcher emits branch / ahead-behind / stash
counts. A `github_repo_prs` fetcher emits a list of open pull requests.

Fetchers come in two flavors, distinguished by how often they run:

- **Cached (async)** — anything that touches I/O: git, HTTP, filesystem
  scans. Splashboard reads the last cached payload to paint instantly, then
  a detached child refreshes the cache in the background for next time. TTL
  is configurable per widget.
- **Realtime (sync, per-frame)** — "right now" values: `clock`,
  `system_cpu`, `system_uptime`, `clock_countdown`. Recomputed on every
  draw tick, no cache. Contract: under 1 millisecond, infallible, no I/O.

You rarely need to think about the flavor — it's a property of the fetcher
you picked. Just know that fresh data after a `cd` is normal (the cache
hadn't refreshed yet) and `splashboard --wait` blocks until every widget
has fetched fresh.

```
  Cached fetcher (most of them — anything that does I/O)
  ──────────────────────────────────────────────────────

    new shell                  ┌─ reads ──▶ cache/        (paint instant)
         │                     │                │
         ▼                     │                │ fresh payload
   splashboard ────────────────┤                ▼
                               │        detached child
                               │                │ runs the fetcher
                               │                ▼
                               └─ writes ──── cache/      (next render
                                                            sees fresh)


  Realtime fetcher  (clock / system_cpu / clock_countdown / pomodoro)
  ──────────────────────────────────────────────────────────────────

    draw tick ──▶ fetch() ──▶ payload ──▶ renderer
    (per frame)   < 1 ms        no cache, no I/O
```

### Renderer — how to draw it

A renderer takes a `Payload` and paints it into a cell of the terminal.
`text_plain` draws plain text. `text_ascii` draws big block / figlet text.
`grid_heatmap` draws a 2D intensity grid. `chart_sparkline` draws inline
mini-bars.

Renderer names follow a `family_variant` convention (`text_plain`,
`text_ascii`, `chart_sparkline`, `grid_heatmap`) so siblings sort together
in the catalog.

A single renderer often accepts multiple shapes of data, and a single shape
is usually drawable by multiple renderers. This is the whole reason the
split exists — same data, different visuals; same visual, different data.

### Layout slot — where on the splash

Rows and their children (`[[row]]` / `[[row.child]]`) define the grid that
widgets occupy. A widget's layout slot determines its width, height,
border, title, and background.

```
 ┌─ splash viewport ─────────────────────────────────────┐
 │ ┌─ row ─────────────────────────────────────────────┐ │
 │ │ ┌── child ─┐  ┌──── child ───┐  ┌──── child ────┐ │ │
 │ │ │ widget A │  │   widget B   │  │   widget C    │ │ │
 │ │ └──────────┘  └──────────────┘  └───────────────┘ │ │
 │ └───────────────────────────────────────────────────┘ │
 │ ┌─ row ─────────────────────────────────────────────┐ │
 │ │                    widget D                       │ │
 │ └───────────────────────────────────────────────────┘ │
 └───────────────────────────────────────────────────────┘

      rows stack vertically · children inside a row stack horizontally
```

See [Configuration](/splashboard/guides/configuration/) for the slot
schema.

## Shapes — the contract between fetcher and renderer

Fetchers and renderers don't talk to each other directly. They both agree
on a **shape** — a small enum that describes the structure of a payload.
Every fetcher declares which shapes it can emit; every renderer declares
which shapes it can draw.

The shapes splashboard ships with today:

| shape | example payload | rendered by |
|---|---|---|
| `Text` | a single string ("14:32") | `text_plain`, `text_ascii`, `animated_typewriter` |
| `TextBlock` | multiple lines (recent commits, welcome notes) | `text_plain`, `list_plain` |
| `Entries` | key/value rows with optional status | `grid_table`, `status_badge` |
| `Ratio` | a 0..=1 value ("year progress 32%") | `gauge_line`, `gauge_circle` |
| `NumberSeries` | `Vec<u64>` (histogram, sparkline) | `chart_sparkline`, `chart_bar` |
| `PointSeries` | `Vec<(f64, f64)>` series | `chart_line`, `chart_scatter` |
| `Bars` | labeled bars | `chart_bar`, `chart_pie` |
| `Image` | PNG / JPEG path | `media_image` |
| `Calendar` | year + month + highlighted days | `grid_calendar` |
| `Heatmap` | 2D intensity grid | `grid_heatmap` |
| `Badge` | short status pill | `status_badge` |
| `Timeline` | chronological entries | `list_timeline` |

Shapes are the **only** coupling between fetchers and renderers. If you add
a new fetcher that emits an existing shape, every compatible renderer just
works. If you add a new renderer that accepts an existing shape, every
fetcher that emits that shape can drive it.

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

## Why this matters in practice

### Same fetcher, different look

The `clock` fetcher emits a `Text` shape. All three of these render the
same data:

```toml
[[widget]]
id = "clock_tiny"
fetcher = "clock"
render = "text_plain"                                    # "14:32"

[[widget]]
id = "clock_big"
fetcher = "clock"
render = { type = "text_ascii", pixel_size = "quadrant" }  # block letters

[[widget]]
id = "clock_figlet"
fetcher = "clock"
render = { type = "text_ascii", style = "figlet", font = "banner" }  # giant
```

No change to the fetcher, three different visuals. Pick the one that fits
the slot you have.

### Same renderer, different source

The `grid_heatmap` renderer accepts a `Heatmap` shape. Any fetcher that
emits one drives it:

```toml
[[widget]]
id = "my_commits"
fetcher = "git_commits_activity"       # your local repo's commit cadence
render = "grid_heatmap"

[[widget]]
id = "my_github"
fetcher = "github_contributions"       # your GitHub contribution graph
render = "grid_heatmap"
```

Same visual treatment, two different data sources.

### Defaults that just work

Every shape has a default renderer, so omitting `render` picks a sensible
one:

```toml
[[widget]]
id = "commits"
fetcher = "git_commits_activity"   # emits NumberSeries
# render omitted → defaults to chart_sparkline (the default for NumberSeries)
```

You only specify `render` when you want to override the default — e.g.
"give me a bar chart instead of a sparkline".

## What about custom widgets?

Splashboard is deliberately a **curated renderer**, not a dashboard
framework. There's no plugin protocol, no subprocess widgets, no
`command = "..."` escape hatch — those invite security and reliability
problems that bound the whole tool.

The one escape hatch for "I want a widget no built-in fetcher provides" is
[ReadStore](/splashboard/guides/cookbook/#readstore-custom-widgets-without-code):
you write a payload file, splashboard deserialises it into any supported
shape, any compatible renderer draws it. Perfect for habit trackers, goal
progress, custom metrics — anything you can shell-script to a file.

Anything deserving of curated UX (rate limits, auth, stateful update) lands
as a built-in fetcher PR instead.

## Next

- [Configuration](/splashboard/guides/configuration/) — the TOML schema
  now that the model makes sense.
- [Reference](/splashboard/reference/matrix/) — every built-in fetcher and
  renderer with its options and compatible shapes. Browse it as a catalog
  once you're thinking in terms of "what shape do I need, who emits it,
  who draws it".
