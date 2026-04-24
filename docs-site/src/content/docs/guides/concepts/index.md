---
title: Concepts
description: The mental model — Widget = Fetcher × Renderer × Layout slot.
---

Once you internalise one equation, every other page clicks into place:

```
Widget = Fetcher × Renderer × Layout slot
```

A **widget** is not a single built-in thing. It's a composition of three
independent pieces that splashboard combines at render time. Splitting
those axes is the whole point: the same data can drive multiple visuals,
the same visual can consume multiple data sources, and each piece evolves
on its own.

```
          one Widget = one composition of three pieces

    Fetcher                Renderer               Layout slot
 ┌────────────┐       ┌──────────────┐      ┌──────────────┐
 │   clock    │──────▶│  text_ascii  │─────▶│ row, height 4│
 │ emits Text │       │ draws blocks │      │   centered   │
 └────────────┘       └──────────────┘      └──────────────┘
      WHAT                  HOW                   WHERE
```

The contract between **Fetcher** and **Renderer** is a small enum called
**Shape**: every fetcher declares which shapes it can emit, every
renderer declares which shapes it can draw, and splashboard compat-checks
them at dispatch time. Swapping either side stays decoupled as long as
the shape matches.

## Deep dives

Each piece has its own page:

- [Widget](/guides/concepts/widget/) — how a widget is
  assembled from config, what its lifecycle looks like across cold /
  warm / animated frames, and how placeholders substitute when things
  go sideways.
- [Shape](/guides/concepts/shape/) — the 12 variants
  splashboard ships today, with the Rust `Body` struct, an example
  payload, the renderers that accept it, and typical fetchers that
  emit it.
- [Fetcher](/guides/concepts/fetcher/) — the Cached
  vs Realtime contract, the Safety classification, options / cache
  keys / sample bodies, and how to add your own.
- [Renderer](/guides/concepts/renderer/) — the trait's
  surface (`accepts` / `animates` / `render`), how alignment and
  empty-state handling work, and the naming convention that keeps
  related renderers clustered in the catalog.

## Why this shape

Three design choices follow directly from the composition:

### Same fetcher, different look

A `clock` fetcher emits `Text`. Three widgets, three visuals, zero
change to the fetcher:

```toml
[[widget]]
id = "clock_tiny"
fetcher = "clock"
render = "text_plain"                                        # "14:32"

[[widget]]
id = "clock_big"
fetcher = "clock"
render = { type = "text_ascii", pixel_size = "quadrant" }    # block letters

[[widget]]
id = "clock_figlet"
fetcher = "clock"
render = { type = "text_ascii", style = "figlet", font = "banner" }
```

### Same renderer, different source

`grid_heatmap` accepts `Heatmap`. Any fetcher that emits one drives it
— same visual treatment, different data:

```toml
[[widget]]
id = "my_commits"
fetcher = "git_commits_activity"       # your local repo's cadence
render = "grid_heatmap"

[[widget]]
id = "my_github"
fetcher = "github_contributions"       # your GitHub graph
render = "grid_heatmap"
```

### Defaults that just work

Every shape has a default renderer, so omitting `render` picks a
sensible one:

```toml
[[widget]]
id = "commits"
fetcher = "git_commits_activity"   # emits NumberSeries
# render omitted → defaults to chart_sparkline (the default for NumberSeries)
```

You only specify `render` when you want to override the default.

## What about custom widgets?

Splashboard is deliberately a **curated renderer**, not a dashboard
framework. There's no plugin protocol, no subprocess widgets, no
`command = "..."` escape hatch — those invite security and reliability
problems that bound the whole tool.

The one escape hatch for "I want a widget no built-in fetcher provides"
is
[ReadStore](/guides/read-store/):
you write a payload file, splashboard deserialises it into any supported
shape, any compatible renderer draws it. Perfect for habit trackers,
goal progress, custom metrics — anything you can shell-script to a file.

Anything deserving of curated UX (rate limits, auth, stateful update)
lands as a built-in fetcher PR instead.

## Next

- [Configuration](/guides/configuration/) — the TOML schema
  now that the model makes sense.
- [Reference](/reference/matrix/) — every built-in fetcher
  and renderer with its options and compatible shapes. Browse it as a
  catalog once you're thinking in terms of "what shape do I need, who
  emits it, who draws it".
