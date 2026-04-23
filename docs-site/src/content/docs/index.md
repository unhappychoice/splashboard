---
title: splashboard
description: Customizable terminal splash rendered on shell startup and cd.
---

A customisable terminal splash rendered on shell startup and on `cd`.
Per-directory `.splashboard/dashboard.toml` means different repos get
different splashes automatically — the killer feature competitor products
(neofetch, fastfetch, starship) don't do.

## Start here

- **[Getting started](/splashboard/guides/getting-started/)** — install,
  wire into your shell, render the first splash.
- **[Concepts](/splashboard/guides/concepts/)** — the one-line mental
  model: Widget = Fetcher + Renderer + Layout slot. Read this before
  configuration.
- **[Configuration](/splashboard/guides/configuration/)** — the TOML
  schema: settings, dashboards, widgets, rows, render options.
- **[Presets](/splashboard/guides/presets/)** — four curated dashboards
  you can copy verbatim.
- **[Themes](/splashboard/guides/themes/)** — six palettes plus per-token
  overrides.
- **[Trust model](/splashboard/guides/trust/)** — per-widget consent for
  network-touching widgets in project-local dashboards.
- **[Cookbook](/splashboard/guides/cookbook/)** — credentials, per-repo
  splashes, custom widgets via ReadStore, light-terminal setup.

## Reference

The [reference section](/splashboard/reference/matrix/) is generated
from code via `cargo xtask`. Every fetcher and renderer has a dedicated
page with its options, compatible shapes, and the colour tokens it
reads. It can't drift from the built-in registries — adding a new
fetcher or renderer and re-running the generator ticks the catalog
mechanically.

Start with the [overview](/splashboard/reference/matrix/) for the shape
× renderer compatibility matrix, then drill into individual fetchers or
renderers as needed.
