---
title: Themes
description: 26 built-in palettes covering every well-known editor theme, per-token overrides, and the reset escape hatch.
---

Every renderer looks up semantic tokens rather than hard-coding colours, so
a single `[theme] preset = "..."` line repaints the whole splash.

splashboard ships with **26 built-in presets** — the signature `default`
palette plus the dark and light staples from across the editor-theme
ecosystem (Catppuccin, Tokyo Night, Rosé Pine, Solarized, Gruvbox,
GitHub, Nord, and more).

## Built-in presets

### Dark themes

| preset | motif |
|---|---|
| `default` | "Splash" — sunrise over deep ocean. Coral hero, cyan-teal accents, navy ground. |
| `tokyo_night` | cool blue-purple, low-saturation. |
| `tokyo_night_storm` | softer storm-cloud variant of Tokyo Night. |
| `nord` | arctic blue-gray, muted accents. |
| `dracula` | dark purple, vibrant neon accents. |
| `gruvbox_dark` | warm earth, retro yellow hero. |
| `catppuccin_mocha` | pastel on dark mauve. |
| `catppuccin_macchiato` | a step lighter than Mocha. |
| `catppuccin_frappe` | warmer pastel mid-dark. |
| `rose_pine` | soho dark with rose and pine highlights. |
| `rose_pine_moon` | moonlit blue-violet variant. |
| `kanagawa` | Hokusai-inspired indigo with autumn accents. |
| `everforest_dark` | woodland green with warm muted accents. |
| `one_dark` | Atom's flagship dark, balanced cyan/red. |
| `solarized_dark` | the precision-engineered classic. |
| `monokai` | punchy magenta and lime on warm gray. |
| `night_owl` | deep midnight blue, calm accents. |
| `synthwave_84` | neon pink and cyan retro grid. |
| `ayu_mirage` | soft slate with bright sky and lime. |
| `material_palenight` | material design dark with lavender. |
| `github_dark` | the github.com dark mode palette. |

### Light themes

Light presets ship with a light `bg` baked in — pick one when your
terminal is light, or override individual tokens with `"reset"` (see
below) to inherit the terminal's own background.

| preset | motif |
|---|---|
| `catppuccin_latte` | pastel on cream. |
| `rose_pine_dawn` | warm cream with pine accents. |
| `solarized_light` | the classic light counterpart. |
| `gruvbox_light` | earthy retro on warm cream. |
| `github_light` | the github.com light mode palette. |

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

## The `"reset"` escape hatch (terminal-native chrome)

Most splashboard presets ship with a baked-in `bg` (and the dark presets
specifically assume a dark terminal). If you want the splash to inherit
your terminal's own background or foreground instead — useful when your
terminal is themed but your splash isn't, or when no built-in light
preset matches your terminal exactly — `"reset"` falls back to
`Color::Reset` for that slot only.

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

Every renderer's [reference page](/reference/matrix/) lists
the tokens it reads. The list is generated from `ColorKey` declarations
in the renderer itself, so it can't drift from what the code actually
uses.
