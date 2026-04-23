---
title: Themes
description: Six built-in palettes, per-token overrides, and the reset escape hatch for light terminals.
---

Every renderer looks up semantic tokens rather than hard-coding colours, so
a single `[theme] preset = "..."` line repaints the whole splash.

splashboard ships with six presets: one signature palette (`default`) and
five community staples.

## Built-in presets

:::note
Screenshots for all six presets rendering the same dashboard are
pending — they ship as part of
[issue #78](https://github.com/unhappychoice/splashboard/issues/78)
once the preset work in
[#77](https://github.com/unhappychoice/splashboard/issues/77) lands.
:::

| preset | motif |
|---|---|
| `default` | "Splash" — sunrise over deep ocean. Coral hero, cyan-teal accents, navy ground. |
| `tokyo_night` | cool blue-purple, low-saturation. |
| `nord` | arctic blue-gray, muted accents. |
| `dracula` | dark purple, vibrant neon accents. |
| `gruvbox_dark` | warm earth, retro yellow hero. |
| `catppuccin_mocha` | pastel on dark mauve. |

## Selecting a preset

```toml
# $HOME/.splashboard/settings.toml

[theme]
preset = "tokyo_night"
```

Omitting `preset` (or setting it to `"default"`) gives the Splash
signature palette.

## Per-token overrides

Any key listed below in `[theme]` wins over the preset value. Unknown keys
are ignored, so adding a new preset or typo never invalidates an existing
config.

```toml
[theme]
preset = "nord"
panel_title = "#ff0088"     # override one token
status_ok = "green"          # named colours work too
```

Accepted colour forms:

- `"#rrggbb"` hex.
- Named colours: `"black"`, `"red"`, `"green"`, `"yellow"`, `"blue"`,
  `"magenta"`, `"cyan"`, `"white"`, and their `"dark_*"` / `"light_*"`
  variants.
- `"reset"` — see below.

## The `"reset"` escape hatch (light terminals)

splashboard presets are tuned for dark terminals — every preset ships a
dark `bg`. On a light terminal, that background clashes with the rest of
your prompt.

`"reset"` falls back to `Color::Reset` for that slot only — i.e. the
terminal's own default. Users on light terminals typically want:

```toml
[theme]
preset = "default"           # or your preference
bg = "reset"                 # inherit terminal bg
bg_subtle = "reset"
text = "reset"               # inherit terminal fg
panel_border = "reset"
```

This keeps the accent tokens (`panel_title`, `status_*`, palettes) from
the preset so the splash still reads cohesively, but drops the heavy
background paint.

## Token reference

Single-colour tokens:

| token | purpose |
|---|---|
| `bg` | viewport background. |
| `bg_subtle` | header / footer / callout bands (`bg = "subtle"`). |
| `text` | primary body text. |
| `text_secondary` | secondary text (timeline details). |
| `text_dim` | chrome text (timeline dates, placeholders). |
| `panel_border` | panel border. |
| `panel_title` | panel title (the palette's hero accent). |
| `status_ok` | healthy / passing status. |
| `status_warn` | degraded / warning status. |
| `status_error` | failing / error status. |
| `accent_today` | calendar "today" marker. |
| `accent_event` | calendar event marker, default scatter dot. |

Multi-colour palettes (TOML arrays):

| token | purpose |
|---|---|
| `palette_series` | series colours cycled by `chart_pie` / `chart_line` / `chart_scatter`. |
| `palette_heatmap` | 5-step intensity ramp for `grid_heatmap`. |

Array override example:

```toml
[theme]
preset = "default"
palette_series = ["#f7768e", "#7aa2f7", "#9ece6a", "#e0af68"]
palette_heatmap = ["#111", "#333", "#666", "#999", "#fff"]
```

## Which tokens does this renderer use?

Every renderer's [reference page](/splashboard/reference/matrix/) lists
the tokens it reads. The list is generated from `ColorKey` declarations
in the renderer itself, so it can't drift from what the code actually
uses.
