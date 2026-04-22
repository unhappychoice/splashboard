# AGENTS.md

Operating notes for AI agents (Claude, Codex, etc.) working on splashboard. Humans reading this are welcome too.

## What splashboard is

A customizable terminal splash rendered on shell startup and on `cd`. One-line install into your shell rc, TOML config, fast cached first-paint with background refresh.

**Killer feature**: per-directory `.splashboard/config.toml` (walk-up discovery). Different repos get different splashes automatically. Competitor products (neofetch, fastfetch, starship) don't do this.

**Positioning contexts** — all three are first-class:

| context | value axis | config source |
|---|---|---|
| self / home | daily delight, ambient info | `$HOME/.splashboard/config.toml` |
| self / project | operational (CI, branch, PRs) | `./.splashboard/config.toml` (per-dir, walk-up) |
| other / project | craft + wow for cloners | `./.splashboard/config.toml` shipped with the repo |

## Core architecture: Shape × Fetcher × Renderer

A **widget** is the composition of three independent pieces:

```
Widget = Fetcher × Renderer × Layout slot
```

Splitting those axes is the whole design. Treat them as separate concerns; resist coupling.

### Shape (`src/render/mod.rs::Shape`, `src/payload.rs::Body`)

The **data-shape contract** between fetchers and renderers. Each `Body` variant corresponds to one `Shape`:

- `Lines` — zero or more text lines
- `Entries` — key/value rows with optional status
- `Ratio` — a single `0..=1` value + optional label
- `NumberSeries` — `Vec<u64>`, histograms / sparklines
- `PointSeries` — `Vec<(f64, f64)>` in one or more series
- `Bars` — labeled bars
- `Image` — path to PNG/JPEG
- `Calendar` — year + month + optional highlighted day + event days
- `Heatmap` — 2D intensity grid with optional thresholds and edge labels

Adding a new shape means:
1. A new `Body` variant + its `*Data` struct in `payload.rs` (serde-serializable).
2. A new `Shape` enum variant in `render/mod.rs`.
3. An entry in `shape_of()`.
4. An entry in `default_renderer_for()`.
5. At least one renderer that lists the new shape in its `accepts()`.

Shapes are the **only** coupling between fetchers and renderers. If you find yourself thinking "my renderer needs the raw fetcher data", add a new shape instead.

### Fetcher (`src/fetcher/`)

Produces a `Payload`. Two flavors:

- **`Fetcher` (cached, async)** — disk cache with TTL, daemon refreshes in background, renderer reads from cache. Right for anything that does I/O: git2, HTTP, filesystem scans.
- **`RealtimeFetcher` (sync, per-frame)** — recomputed on every draw tick, no cache at all. Right for "right now" values: `clock`, `system_cpu`, `system_uptime`, `clock_countdown`, `pomodoro`. Contract: < 1ms, infallible, no I/O. If you want to put `reqwest` in a `RealtimeFetcher`, it's not realtime — it's cached.

Key invariants:

- Each fetcher declares its supported shapes via **`fn shapes(&self) -> &[Shape]`**. Multi-shape fetchers (`clock`, `read_store`) branch on `ctx.shape` inside `fetch` / `compute`; the runtime validates the config-requested shape against the list before dispatch and renders a placeholder on mismatch instead of crashing. Single-shape fetchers just return a one-element slice.
- Each fetcher declares a **`Safety`** class:
  - `Safe` — renders even in untrusted local configs. Local-only reads, or fixed-host authenticated network (the token only leaves to a known host).
  - `Network` — trust-gated when the URL or query is config-provided (rss, calendar, any fetcher that takes a user URL).
  - `Exec` — **no longer supported**. Plugin protocol (#5) and command widget (#20) are closed. Don't reintroduce.

- `ReadStore` (`src/fetcher/read_store.rs`) is the escape hatch for "I want a custom widget": user writes `$HOME/.splashboard/store/<id>.<ext>`, splashboard deserializes per the declared shape. Always `Safe` (fixed path, no traversal).

- Fetchers declare their output shape(s) explicitly via `shapes()`. Renderers are compat-checked against the emitted `Body` variant at dispatch time.

- Both kinds register into the shared `Registry` via `with_builtins()`. Lookup is name-keyed; realtime and cached live in the same namespace, same name = collision (last one wins).

### Renderer (`src/render/`)

Consumes a `Payload` + `RenderOptions` and draws into a ratatui `Frame`. Each renderer declares:

- `name()` — used in config (`render = "heatmap"` or `render = { type = "heatmap", align = "center" }`)
- `accepts()` — list of `Shape` it can render. A renderer can accept multiple shapes if it makes sense (rare).
- `animates()` — `true` if it produces different output on repeated calls within a single draw cycle. Affects whether the runtime extends the 2-second animation window to let the motion play.
- `render(frame, area, body, opts)` — do the drawing.

Key invariants:

- **One shape can feed multiple renderers**. `Lines` → `simple`, `ascii_art`, `animated_typewriter`. That flexibility is the point; resist "one shape, one renderer".

- **Empty-state handling is centralized**. `render::render_payload` short-circuits any body that `is_empty_body()` considers empty to the shared "nothing here yet" placeholder. Don't bake empty handling into individual renderers.

- **Unknown renderer or shape/renderer mismatch** renders an in-band error string, never panics. Users must be able to see what's misconfigured without crashing the splash.

- **Alignment**. `RenderOptions.align` (`left` / `center` / `right`) is honored by renderers where it makes sense (text, ascii_art, heatmap). Structural renderers (table, gauge, charts) ignore it.

## Trust model (`src/trust.rs`)

Per-widget gate, not per-file. Safe widgets always render; Network widgets are replaced with a `🔒 requires trust` placeholder when the local config is untrusted.

- Local configs must be `splashboard trust`-ed before their Network widgets run. Global config is `ImplicitlyTrusted` (user's own authority).
- Trust is `(canonicalized path, sha256)` — editing the file revokes trust automatically.
- TOCTOU-safe: the trust-sensitive callers use `load_config_and_hash()` which reads bytes once.
- Escape hatch: `SPLASHBOARD_TRUST_ALL=1`.
- `Exec` class is closed (plugin / command-widget dropped in #5 / #20). The gate exists for future-proofing but the Exec arm is currently unreachable.

Details: see the `project_trust_model.md` memory (auto-loaded) for the full threat model and hardening rules.

## Filesystem layout

All splashboard state lives under **`$HOME/.splashboard/`** (no XDG paths, same on Linux/macOS/Windows):

```
$HOME/.splashboard/
├── config.toml        # global config
├── trust.toml         # trust store (path + sha256 entries)
├── cache/             # per-widget cache (key.json + key.lock)
└── store/             # ReadStore files — $HOME/.splashboard/store/<id>.<ext>
```

Overridable via `SPLASHBOARD_HOME` env var (for tests, CI, relocatable installs).

Per-directory configs stay in the repo: `./.splashboard/config.toml` or `./.splashboard.toml` (walk-up discovery starting from CWD).

## Widget / renderer / fetcher catalog: #41

Issue #41 is the living roadmap. All speculative widget/fetcher/renderer ideas are checkboxes there — **no separate issue per candidate**. A PR that ships something ticks the box. Things that DO get their own issue: cross-cutting features (CLI subcommands, theme system).

Prioritization rubric (see #41 for the full version):
1. Renderer primitives unlock many fetchers → highest leverage.
2. Daily-driver coverage (fresh install looks satisfying).
3. Per-dir killer feature (cd into a project shows more than without splashboard).
4. Dedicated built-ins beat ReadStore for curated UX; ReadStore is the ad-hoc escape hatch.

## Conventions

### Code style (from repo CLAUDE rules)

- English in source / commits / docs.
- Public items at top of files.
- Prefer `map` / `filter` / `reduce` over imperative loops.
- Single responsibility, small functions (~10–15 lines), small files (~100 lines where practical).
- Minimal comments — well-named functions do the explaining. Add a comment only when the *why* is non-obvious: a hidden constraint, a subtle invariant, a workaround.
- Tests live next to the code in `#[cfg(test)] mod tests` blocks. Integration / cross-module tests reuse `src/render/test_utils.rs`.

### Git / PR style

- Conventional commit messages (`feat(scope): ...`, `fix(scope): ...`, `chore(...)`, `refactor(...)`).
- Every PR gets a `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer.
- **Merge strategy**: always `--merge` (merge commit). Never rebase or squash — this is a hard project preference captured in memory.
- PRs include a Summary + Test plan section. Link the catalog issue (#41) with the checkbox that's being ticked.

### Testing

- `cargo test` must be green before a PR is opened. `cargo clippy --all-targets -- -D warnings` too.
- Tests that mutate process env (`SPLASHBOARD_HOME` is the notable one) must take `paths::TEST_ENV_LOCK` to serialize with other tests.
- Rendering tests use `src/render/test_utils.rs` — `render_to_buffer*()` helpers return a `Buffer` you can scan for expected cells/symbols.

## Rejected designs (don't reintroduce)

- **Plugin protocol (#5)** — closed. Splashboard is a curated splash renderer, not a dashboard framework. Subprocess plugins can't be bounded in blast radius (a generic `http_fetch` plugin breaks every mitigation). Custom widgets land as built-in PRs or use ReadStore.
- **Command widget (#20)** — closed. No `command = "..."` fetcher, in any config scope. Local-vs-global rule carve-outs are not worth the footgun potential or the threat-model complexity.
- **Screensaver sub-mode** — out of scope. Animation lives within the existing 2-second ANIMATION_WINDOW; a persistent idle loop is a different product.
- **XDG paths via the `dirs` crate** — migrated away. One user-visible `$HOME/.splashboard/` for all platforms, because `~/Library/Application Support/splashboard/` is a surprising place for a CLI tool's state.

## When in doubt

- **"Which fetcher does this belong in?"** — same underlying read, different presentation = same fetcher, different shape (add the variant to `shapes()` and branch in `fetch` / `compute`). Genuinely different data = different fetcher.
- **"Which renderer?"** — does an existing one accept this shape? Use it. Need different visuals? Register a new renderer that also accepts the shape; don't invent a new shape.
- **"Is this Safe or Network?"** — ask "can config control where the traffic goes?". If yes → Network. If the URL is hardcoded in the struct → Safe, even with auth (the token only leaves to a known host).
- **"Realtime or cached?"** — can this fetch ever take > 1ms or ever fail? Cached. Otherwise realtime.
- **"ReadStore or built-in?"** — does the widget deserve curated UX (parsing, auth, rate limits, stateful update)? Build it. Is it "a number the user can write to a file"? ReadStore.
