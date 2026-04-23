---
title: Cookbook
description: Recipes for common customizations — credentials, per-repo splashes, custom widgets, theming.
---

Small recipes that stitch together the pieces from the other guides.

## GitHub credentials

`github_*` fetchers need a token with read access to the resources they
query. splashboard reads `GH_TOKEN` first, then falls back to
`GITHUB_TOKEN`.

```bash
# In ~/.zshrc / ~/.bashrc — export before the splashboard init snippet.
export GH_TOKEN="ghp_..."
```

If you already have the `gh` CLI configured and don't want the token in
an rc file, a typical setup is:

```bash
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
trust` — see the [trust guide](/splashboard/guides/trust/).

## ReadStore — custom widgets without code

Need a widget no fetcher currently provides? Write the payload to a file
and point a `read_store` widget at it.

```toml
# dashboard.toml
[[widget]]
id = "habit"
fetcher = "read_store"
file_format = "json"
render = "gauge_line"
```

splashboard reads from
`$HOME/.splashboard/store/habit.json` (the filename matches the widget
`id`). Write anything there — a cron job, a shell function, a
post-commit hook — and the splash picks it up.

ReadStore understands every non-dynamic shape: `Text`, `TextBlock`,
`Entries`, `Ratio`, `NumberSeries`, `PointSeries`, `Bars`, `Image`,
`Calendar`, `Heatmap`. The shape you get is the one the renderer
accepts; `gauge_line` above takes a `Ratio`, so:

```json
{
  "ratio": 0.6,
  "label": "habit · 6 of 10 days"
}
```

### Habit tracker example

A tiny shell script that ticks today off and writes the ratio:

```bash
#!/usr/bin/env bash
# ~/.local/bin/habit-tick
store="$HOME/.splashboard/store/habit.json"
touch "$HOME/.splashboard/habit.log"
date +%Y-%m-%d >> "$HOME/.splashboard/habit.log"
sort -u -o "$HOME/.splashboard/habit.log" "$HOME/.splashboard/habit.log"

count=$(grep -c "^$(date +%Y-%m)" "$HOME/.splashboard/habit.log")
days=$(date +%d)
ratio=$(awk "BEGIN { printf \"%.3f\", $count / $days }")

cat > "$store" <<EOF
{ "ratio": $ratio, "label": "this month · $count of $days days" }
EOF
```

Call `habit-tick` on the days you did the thing; the splash reflects it
next render.

### Goal progress

Same pattern with a yearly ratio and a fixed target:

```json
{
  "ratio": 0.47,
  "label": "reading · 14 of 30 books"
}
```

Keep the file `$HOME/.splashboard/store/reading.json`; point a
`read_store` widget at it with `id = "reading"`.

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
[reference page](/splashboard/reference/matrix/), so you can work
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
