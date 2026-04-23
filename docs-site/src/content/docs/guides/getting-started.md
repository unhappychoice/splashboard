---
title: Getting started
description: Install splashboard, wire it into your shell, and render your first splash.
---

splashboard renders a customizable TUI splash on every new shell and on `cd`.
This page walks through installing the binary, wiring it into your shell, and
verifying the first splash. It assumes nothing more than a working terminal.

## Install

splashboard is distributed on crates.io. A stable Rust toolchain is the only
prerequisite.

```bash
cargo install splashboard
```

Prebuilt binaries and a Homebrew tap are planned (see
[issue #16](https://github.com/unhappychoice/splashboard/issues/16)); until
then, `cargo install` is the supported path.

## Wire it into your shell

`splashboard init <shell>` prints a snippet that calls `splashboard` on every
new interactive shell and re-renders on `cd`. Append it to your rc file once:

```bash
# bash
splashboard init bash >> ~/.bashrc

# zsh
splashboard init zsh  >> ~/.zshrc

# fish
splashboard init fish >> ~/.config/fish/config.fish

# PowerShell (Windows)
splashboard init powershell >> $PROFILE
```

Open a new shell — you should see a splash render.

## The first splash

A fresh install ships no widgets: the built-in home and project dashboard
fallbacks are intentionally empty so you see nothing instead of something
generic. Drop widgets in to make it yours:

```toml
# $HOME/.splashboard/home.dashboard.toml

[[widget]]
id = "clock"
fetcher = "clock"
render = { type = "text_ascii", pixel_size = "quadrant", align = "center" }

[[row]]
height = { length = 4 }
  [[row.child]]
  widget = "clock"
```

Open a new shell or run `splashboard` directly. You'll see a large-format
clock centered in the splash.

From here:

- [Concepts](/splashboard/guides/concepts/) — the mental model (Widget =
  Fetcher + Renderer + Layout slot). Read this next if the TOML above
  looks a bit opaque; it unlocks the rest of the docs.
- [Configuration](/splashboard/guides/configuration/) — the full TOML
  schema (settings, dashboards, widgets, rows, render options).
- [Presets](/splashboard/guides/presets/) — curated dashboards you can
  adopt verbatim.
- [Themes](/splashboard/guides/themes/) — six built-in palettes plus
  per-token overrides.
- The [reference](/splashboard/reference/matrix/) — the complete list of
  fetchers and renderers with their options and compatible shapes.

## Troubleshooting

**Nothing renders.** splashboard deliberately opts out of rendering in
contexts where a splash would be noise rather than signal. The splash skips
rendering when any of these is true:

- `stdout` is not a terminal (piped, redirected, non-interactive).
- `TERM=dumb`.
- Any of `CI`, `SPLASHBOARD_SILENT`, or `NO_SPLASHBOARD` is set.
- The terminal is smaller than 40×16.

Unset the variable or widen the terminal and try again.

**A widget renders `🔒 requires trust`.** The widget is classified as
`Network` and the local dashboard it came from has not been trusted yet. See
the [trust model guide](/splashboard/guides/trust/) for the consent flow —
`splashboard trust` is usually what you want.

**A GitHub widget renders an error.** `github_*` fetchers need a token with
read access. Export `GH_TOKEN` (or `GITHUB_TOKEN`) before starting the shell.
See the [cookbook](/splashboard/guides/cookbook/#github-credentials) for the
recommended place to put it.

**I want to see fresh data, not cached.** Run `splashboard --wait` once. The
cached-first model normally paints instantly from the last refresh; `--wait`
blocks until every widget has fetched fresh.
