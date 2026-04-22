---
title: splashboard
description: Customizable terminal splash rendered on shell startup and cd.
---

A customisable terminal splash rendered on shell startup and on `cd`. Per-directory
`.splashboard/config.toml` means different repos get different splashes automatically.

## Browse the catalog

- [Reference overview](/splashboard/reference/matrix/) — fetcher index, renderer index, and the
  shape × renderer compatibility matrix.
- Each fetcher and renderer has a dedicated page with its options schema and the widgets it
  pairs with.

## Source of truth

Reference pages under **Reference** are generated from code via `cargo xtask`. They can't drift
from the built-in registries — adding a fetcher or renderer and re-running the generator ticks
the catalog mechanically.
