---
title: Renderer
description: The Renderer trait — accepts, animates, options, alignment, empty-state handling, and the naming convention that keeps the catalog sorted.
---

A renderer takes a `Payload` body and paints it into a ratatui cell.
It's the HOW half of the composition — it doesn't know who produced
the body, just that its shape is one it accepts.

## The trait

```rust
pub trait Renderer: Send + Sync {
    fn name(&self) -> &str;
    fn accepts(&self) -> &[Shape];
    fn animates(&self) -> bool { false }
    fn option_schemas(&self) -> &[OptionSchema] { &[] }
    fn color_keys(&self) -> &[ColorKey] { &[] }
    fn render(&self, frame, area, body, opts, theme, registry);
    fn natural_height(&self, body, opts, theme, max_width, registry) -> u16 { 1 }
}
```

Six methods that matter, each doing one thing.

### `name()` — the identifier

Used in config (`render = "grid_heatmap"`) and in the catalog. Names
follow a `family_variant` convention so siblings cluster: `text_plain`
/ `text_ascii`, `gauge_line` / `gauge_circle`, `chart_bar` /
`chart_line` / `chart_pie` / `chart_scatter` / `chart_sparkline`,
`grid_table` / `grid_calendar` / `grid_heatmap`, `list_plain` /
`list_timeline`, `status_badge`, `media_image`,
`animated_typewriter` / `animated_postfx` /
`animated_figlet_morph` / `animated_boot` / `animated_scanlines` /
`animated_splitflap` / `animated_wave`.

The convention extends beyond renderers — theme token names, preset
names, fetcher names all follow it. No standalone public tokens that
could later need a family prefix.

### `accepts()` — compatibility

The list of shapes this renderer can draw. The runtime's dispatcher
compat-checks the body's shape against this before calling `render`.
An unknown renderer name or a mismatch draws an in-band error; it
never panics. Users see what's misconfigured rather than a dead
splash.

A single renderer can accept multiple shapes — `text_plain` draws
both `Text` and `TextBlock`, `grid_table` draws `Entries` and
`Badge`, `chart_bar` draws both `NumberSeries` and `Bars`. The
accepts list is the source of truth for the compatibility matrix on
the [reference overview](/reference/matrix/).

### `animates()` — runtime hinting

`true` if the renderer produces different output on repeated calls
within a single draw cycle. Consulted by the runtime: any `true`
upgrades the draw phase from a one-shot paint to a 2-second
multi-frame loop so the animation actually plays.

`animated_typewriter` (character-by-character reveal),
`animated_postfx` (tachyonfx-powered sweep / fade / coalesce /
`stagger_reveal` / `matrix_rain` / `particle_burst` / `bounce_in` /
`elastic_in` / `checkerboard_in` / `neon_flash` / `glitch_in`),
`animated_figlet_morph` (figlet-font sequence crossfade),
`animated_boot` (boot-log scroll then hero),
`animated_scanlines` (CRT-style horizontal scanline sweep),
`animated_splitflap` (departure-board per-cell letter cycling), and
`animated_wave` (vertical crest travels left-to-right) return
`true`. Everything else stays `false` — the splash paints once and
exits.

### `render(…)` — the draw

```rust
fn render(
    &self,
    frame: &mut Frame,
    area: Rect,
    body: &Body,
    opts: &RenderOptions,
    theme: &Theme,
    registry: &Registry,
);
```

The meat. Paints inside `area` using ratatui widgets, reading colours
from `theme` and honouring the renderer-specific fields inside
`opts`. The `registry` is threaded through so composite renderers
(like `animated_postfx`) can dispatch to an inner renderer by name.

### `option_schemas()` / `color_keys()` — docs metadata

`option_schemas()` declares the renderer-specific fields (`style`,
`pixel_size`, `align`, `font`) the `render` inline table accepts.
`color_keys()` declares the theme tokens the renderer reads. Both are
consumed at docs-generation time — `cargo xtask` emits one reference
page per renderer listing every knob and every token it touches,
straight from the declaration. The catalog can't drift from the code.

### `natural_height(…)` — auto-sized rows

Most renderers draw single-line or fixed output and stick with the
default `1`. A row with `height = "auto"` asks its child's renderer
how tall it wants to be, given the row's width. `text_ascii` overrides
to report the wrapped figlet block height so multi-word heroes get a
row sized to fit; `animated_postfx` / `animated_boot` /
`animated_scanlines` / `animated_splitflap` / `animated_wave`
delegate to their inner renderer via the `registry`, and
`animated_figlet_morph` asks `text_ascii` for the tallest height
across its font sequence so earlier phases never clip.

## `RenderOptions`

The `render` field in config is either a bare string (renderer name)
or an inline table. The table carries renderer-specific fields:

```toml
# Bare string form — uses all defaults.
render = "text_plain"

# Full form — type + options.
render = { type = "text_ascii", style = "figlet", font = "banner", align = "center" }
```

Common fields:

- `type` — the renderer name (required in the full form).
- `align` — `"left"` / `"center"` / `"right"`. Honoured by renderers
  where horizontal alignment makes sense (`text_plain`, `text_ascii`,
  `grid_heatmap`). Structural renderers (`grid_table`, gauges,
  charts) ignore it — their layout is intrinsic.
- `color` — a theme token override (e.g. `"panel_title"` for the coral
  accent). Optional; defaults to the renderer's declared `color_keys`.

Renderer-specific fields vary (`style`, `pixel_size`, `font`,
`max_items`, `bullet`, `date_format`, `effect`, `duration_ms`, …) and
are documented on each renderer's reference page.

## Empty-state handling

Bodies with no data never reach `render`. The dispatcher
(`render::render_payload`) short-circuits any body that
`is_empty_body(&body)` considers empty to the shared "nothing here
yet" placeholder:

- `Text` with empty `value`
- `TextBlock` with no lines or all blank
- `Entries`, `NumberSeries`, `PointSeries`, `Bars`, `Timeline` —
  empty collection
- `Image` with empty path
- `Heatmap` with no cells
- `Badge` with empty label

`Ratio` and `Calendar` are exceptions — `0%` and "this month" are
both legitimate data.

Centralising this means every renderer sees the same "no data"
behaviour. Don't bake empty handling into individual renderers.

## Animation integration

Animated renderers wrap an inner renderer rather than duplicating its
layout logic:

```toml
[[widget]]
id = "hero"
fetcher = "system"
render = { type = "animated_postfx", inner = "text_ascii", effect = "particle_burst",
           duration_ms = 1500, style = "figlet", font = "ansi_shadow", align = "center" }
```

The outer `animated_postfx` carries the effect parameters
(`effect`, `duration_ms`) and every option the inner renderer reads
(`style`, `font`, `align`, …). At dispatch time the outer animator
calls the inner renderer for the frozen frame, then applies the
effect shader on top. The final rested frame is whatever the inner
renderer would have drawn without the wrapper.

`animated_postfx` ships with a menu of effect names (full list in the
[renderer reference](/reference/renderers/animated/animated_postfx/)):

- `fade_in` / `fade_out` / `dissolve` / `coalesce` / `hsl_shift` — stock
  tachyonfx reveals.
- `sweep_in` / `sweep_in_right` / `sweep_in_down` / `sweep_in_up` /
  `slide_in*` — directional wipes.
- `stagger_reveal` / `stagger_reveal_radial` — per-cell diagonal /
  radial fade-in, calm enough for daily-use presets.
- `matrix_rain` — random glyphs rain and dissolve into the underlying
  render.
- `particle_burst` — particles radiate in from the centre and resolve
  into the inner render. The default for `home_splash`'s hero.
- `bounce_in` / `elastic_in` — bounce / spring timing curves for a
  playful arrival.
- `checkerboard_in` — tile-by-tile fade-in on a checker grid.
- `neon_flash` — a bright hue / lightness pulse that settles back into
  the theme colour; for a "neon sign warming up" vibe.
- `glitch_in` — scrambles a fraction of cells with broken-signal glyphs
  during the window, then releases into the clean inner render.

Five sibling renderers bring their own timeline instead of a tachyonfx
pattern:

- `animated_figlet_morph` — steps `text_ascii` through a sequence of
  figlet fonts (`small` → `banner` → `ansi_shadow` by default) with a
  short crossfade between phases. The final font is the resting frame.
- `animated_boot` — scrolls a list of `[ OK ] …` boot-log lines during
  the first ~70% of the window, then hands off to the inner renderer
  for the resting frame. Best on tall hero cells (8+ rows).
- `animated_scanlines` — CRT-style horizontal scanline sweeps down the
  widget cell, revealing the inner render as it passes. Rows below the
  scanline stay blank until the line reaches them.
- `animated_splitflap` — departures-board aesthetic; every non-blank
  cell cycles through `A-Z / 0-9 / punctuation` and lands on its final
  glyph at a position-dependent settle time. Left columns land first.
- `animated_wave` — a bright vertical crest sweeps left-to-right across
  the cell; columns ahead of the crest stay blank, columns behind are
  revealed, and the crest column itself is highlighted with the accent.

The inner renderer keeps its own option schema; options pass through
untouched.

## Defaults per shape

Omitting `render` in config picks the shape's default renderer:

| shape | default |
|---|---|
| `Text` | `text_plain` |
| `TextBlock` | `text_plain` |
| `Entries` | `grid_table` |
| `Ratio` | `gauge_circle` |
| `NumberSeries` | `chart_sparkline` |
| `PointSeries` | `chart_line` |
| `Bars` | `chart_bar` |
| `Image` | `media_image` |
| `Calendar` | `grid_calendar` |
| `Heatmap` | `grid_heatmap` |
| `Badge` | `status_badge` |
| `Timeline` | `list_timeline` |

Only write `render = ...` when you want to override the default.

## Adding a renderer

1. New file under `src/render/` named for the renderer
   (`src/render/chart_radar.rs`).
2. Define a unit struct (`pub struct ChartRadarRenderer;`) and
   implement `Renderer` on it.
3. Pick a name: `family_variant`. Reuse an existing family if your
   renderer is a cousin (`chart_*` / `grid_*` / `gauge_*` / `list_*`);
   start a new family only if nothing fits.
4. Honour `align` where alignment makes semantic sense.
5. Declare `option_schemas()` and `color_keys()` so the reference
   page generates automatically.
6. Register in `Registry::with_builtins()` in `src/render/mod.rs`.
7. Add rendering tests via `src/render/test_utils.rs` — scan the
   resulting `Buffer` for expected cells/symbols.
8. Run `cargo xtask` to update the reference matrix and per-renderer
   page.

The dispatcher handles compatibility / empty-state / error display;
your renderer only needs to implement `render` for the happy path.
