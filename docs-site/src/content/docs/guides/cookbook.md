---
title: Cookbook
description: Recipes for common customizations — credentials, per-repo splashes, custom widgets, theming.
---

Small recipes that stitch together the pieces from the other guides.

## GitHub credentials

`github_*` fetchers need a token with read access to the resources they
query. splashboard reads `GH_TOKEN` first, then falls back to
`GITHUB_TOKEN`.

The recommended place is `$HOME/.splashboard/secrets.toml` — a flat TOML
map of `KEY = "value"` pairs that splashboard exports as process env at
startup. No shell rc edit, no leakage into other tools.

```toml
# $HOME/.splashboard/secrets.toml
GH_TOKEN      = "ghp_..."
TODOIST_TOKEN = "..."          # any fetcher that reads an env var works the same way
```

`splashboard install` git-ignores this file under
`$HOME/.splashboard/.gitignore` so dotfiles-in-git users don't commit it
by accident. Shell env always wins, so an ad-hoc
`GH_TOKEN=... splashboard ...` override still works. See the
[`secrets.toml` reference](/guides/configuration/#secretstoml) for the
denylist and full rules.

If you'd rather keep the token in your shell rc — e.g. because other
tools also read it — that still works. Export before the splashboard init
snippet:

```bash
# ~/.zshrc / ~/.bashrc
export GH_TOKEN="ghp_..."
# or, if you have the `gh` CLI configured:
export GH_TOKEN="$(gh auth token 2>/dev/null)"
```

No token? The `github_*` widgets render a "GH_TOKEN / GITHUB_TOKEN not
set" error instead of breaking the rest of the splash.

## A different splash per repo

The killer feature. Ship a per-dir dashboard with any repo:

```
your-repo/
├── .git/
├── .splashboard/
│   └── dashboard.toml      # renders when `cd your-repo`
├── src/
└── ...
```

Or a single-file variant: `your-repo/.splashboard.toml`.

The per-dir dashboard only activates at the repo root itself — `cd
your-repo/src/` falls back to the home dashboard, so you don't see the
project splash repaint on every sub-cd.

If the repo ships no `.splashboard/`, splashboard still recognises the
context: at a git repo root, it loads
`$HOME/.splashboard/project.dashboard.toml` instead of the home one.
Drop widgets in there to get a single "every repo root" splash without
each repo carrying a config.

Network widgets in a per-dir dashboard stay gated behind `splashboard
trust` — see the [trust guide](/guides/trust/).

## Custom widgets without code

Use [ReadStore](/guides/read-store/) — write a payload
file, pair it with a renderer, splashboard draws it. No code, no
subprocess. Good for habit trackers, goal progress, and anything you
can shell-script to a file.

## Making your own theme

Start from a preset that's close, then override the tokens that don't
fit.

```toml
# settings.toml
[theme]
preset = "tokyo_night"

# Repaint the panel titles with a warm coral, keep everything else.
panel_title = "#f7768e"

# Swap the series palette for a two-colour cycle (nice for on/off charts).
palette_series = ["#7aa2f7", "#565f89"]
```

Every renderer declares the tokens it reads on its
[reference page](/reference/matrix/), so you can work
backward: "I want this widget to pop more" → open the widget's page →
see which token it uses → override that.

## Light-terminal setup

splashboard presets are tuned for dark terminals. On a light terminal,
fall back to the terminal's own background and foreground with `"reset"`:

```toml
[theme]
preset = "default"
bg = "reset"
bg_subtle = "reset"
text = "reset"
text_secondary = "reset"
panel_border = "reset"
```

Accent tokens (`panel_title`, `status_*`, palettes) stay from the
preset, so the splash keeps its identity without the heavy background
paint.

## Opting out

The splash skips rendering when `stdout` isn't a terminal, `TERM=dumb`,
or any of `CI`, `SPLASHBOARD_SILENT`, or `NO_SPLASHBOARD` is set. Also
skipped below 40×16.

To mute just the current shell:

```bash
export SPLASHBOARD_SILENT=1
```

To trade snappiness for freshness (block the initial paint until every
widget has fetched):

```bash
splashboard --wait
```

Or persistently in settings:

```toml
[general]
wait_for_fresh = true
```
