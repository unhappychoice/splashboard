---
title: Fetcher
description: The Cached vs Realtime contract, Safety classification, options, cache keys, and how to add a fetcher.
---

A fetcher produces a `Payload` for a widget to render. It's the WHAT
half of the composition — it doesn't know who will draw its output,
just that it must return a body whose shape it declared.

Fetchers come in two flavors, distinguished by how often they run and
what I/O they may do:

- **Cached** — touches I/O (git, HTTP, filesystem). Runs on a detached
  child, writes to the on-disk cache, next render reads from the cache.
- **Realtime** — the value *is* "right now". Runs synchronously on every
  draw tick. Must be under a millisecond and infallible.

Pick flavor by asking *"can this fetch ever take more than a
millisecond or ever fail?"* If yes, it's Cached. If genuinely no —
wall-clock math, in-memory counters — it's Realtime.

## The Cached contract

```rust
#[async_trait]
pub trait Fetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    fn shapes(&self) -> &[Shape];
    fn default_shape(&self) -> Shape { self.shapes()[0] }
    fn option_schemas(&self) -> &[OptionSchema] { &[] }
    fn sample_body(&self, shape: Shape) -> Option<Body> { … canonical fallback … }
    fn cache_key(&self, ctx: &FetchContext) -> String { default_cache_key(…) }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError>;
}
```

Cached fetchers power most of the catalog: `git_*`, `github_*`,
`weather`, `quote_of_day`, and anything user-written via
`basic_read_store`. They run off the critical render path, so the
splash paints instantly with whatever's in the cache and the fresh
payload lands on the next `cd`.

### `fetch(ctx)`

Called from a detached child via `splashboard --fetch-only`. Receives
a `FetchContext` carrying the widget's `format`, `options`, `shape`,
and a per-fetch timeout. Returns `Result<Payload, FetchError>`; errors
surface as an in-band placeholder in the widget's slot on the next
render.

### Cache behaviour

The cache key is `fetcher.cache_key(ctx)`. The default covers the
common case (`name + format`); fetchers whose output depends on more
(cwd, repo, URL) override it to include those. Two widgets that share
a cache key share a payload — which is how `clock` with the same
`format` in two different widgets only refreshes once.

Cache files live at `$HOME/.splashboard/cache/<key>.json` with a
sibling `<key>.lock` for the refresh-coalescing lock. TTL is
configurable per widget in settings (`[general]` / per-widget in
config).

### `sample_body(shape)`

A representative payload body used for docs previews (and the
dashboard snapshots under `docs-site/src/assets/rendered/`). Defaults
to `samples::canonical_sample(shape)`; fetchers override to surface
something closer to real output (`"main +2 ◆3"` for `git_status`,
`"14:32"` for `clock`). Only touched by docs / tests — the runtime
always goes through the real `fetch` / `compute` path.

## The Realtime contract

```rust
pub trait RealtimeFetcher: Send + Sync {
    fn name(&self) -> &str;
    fn safety(&self) -> Safety;
    fn shapes(&self) -> &[Shape];
    fn default_shape(&self) -> Shape { self.shapes()[0] }
    fn option_schemas(&self) -> &[OptionSchema] { &[] }
    fn sample_body(&self, shape: Shape) -> Option<Body> { … }
    fn compute(&self, ctx: &FetchContext) -> Payload;
}
```

Realtime fetchers are the "per-frame" crowd: `clock`, `clock_derived`,
`clock_ratio`, `clock_timezones`, `clock_almanac`, `system_*`,
`quote_of_day`. `compute` is called on every draw tick with the widget's
context; the output is used immediately without cache.

The contract is strict: **< 1 ms, infallible, no I/O.** Anything that
could block the render thread (disk reads, HTTP, shelling out) belongs
in a cached fetcher.

## `FetchContext`

Same context type for both flavors, populated from the widget config:

```rust
pub struct FetchContext {
    pub widget_id: String,
    pub format: Option<String>,       // e.g. "%H:%M" for clock
    pub timeout: Duration,            // per-fetch ceiling (Cached only)
    pub file_format: Option<String>,  // ReadStore payload encoding
    pub shape: Option<Shape>,         // target shape for multi-shape fetchers
    pub options: Option<toml::Value>, // deserialized by each fetcher
}
```

Multi-shape fetchers (`clock`, `basic_read_store`) branch on
`ctx.shape` inside `fetch` / `compute` to pick which variant to emit.
Single-shape fetchers ignore it.

## Safety classes

Every fetcher declares a `Safety` at compile time. The trust gate
consults it to decide whether the widget runs or renders a
`🔒 requires trust` placeholder in an untrusted local config:

| class | rule |
|---|---|
| `Safe` | Always runs. Local-only reads, or network calls with hard-coded hosts (`api.github.com` in the fetcher struct). The credential can only leave to the known destination. |
| `Network` | Runs only after `splashboard trust` for local project dashboards. For configs whose URL or query is user-supplied (RSS, calendar feeds, custom HTTP). The config can steer traffic to an arbitrary host — which is the entire threat. |
| `Exec` | Subprocess widgets. **Permanently closed** — splashboard is a curated renderer, not a shell script host. The variant exists so the trust gate stays future-proof. |

Home-scoped configs (`$HOME/.splashboard/*.dashboard.toml`) are
implicitly trusted — you own HOME, so anything you put there is
authoritative. The trust gate only applies to project-local dashboards
that travel with a cloned repo. See
[Trust model](/splashboard/guides/trust/) for the full threat model.

### What counts as "fixed host"?

`github_*` fetchers talk to `api.github.com`. The URL is hard-coded in
the fetcher struct; config can't redirect the token elsewhere. That's
`Safe` even though authentication is involved — the credential stays
with the known destination.

A hypothetical `http_fetch` that takes a user-supplied URL is
`Network`, even with no credentials — *the config controls where
traffic goes*, which is what the classification is checking.

## Options

Fetchers accept typed options via `[widget.options]`. The fetcher
defines an `Options` struct with serde defaults and declares its
schema via `option_schemas()` so the docs generator can list every
knob per-fetcher automatically:

```rust
#[derive(Deserialize)]
struct Options {
    #[serde(default = "default_count")]
    count: usize,
    #[serde(default)]
    relative: bool,
}
```

```toml
[[widget]]
id = "commits"
fetcher = "git_recent_commits"
  [widget.options]
  count = 5
  relative = true
```

Unknown keys are ignored — adding a new option never breaks an
existing config, and typos silently fall back to defaults rather than
erroring.

## Escape hatches

Two built-in fetchers cover custom widgets without code:

- **`basic_static`** — ships a constant body. The widget's `format`
  field is the literal value. Used for labels / section headers /
  decorative strings.

  ```toml
  [[widget]]
  id = "greeting_prefix"
  fetcher = "basic_static"
  format = "good "
  render = { type = "text_plain", align = "right" }
  ```

- **`basic_read_store`** — reads
  `$HOME/.splashboard/store/<id>.<ext>` and deserialises it as the
  [shape](/splashboard/guides/concepts/shape/) the paired renderer
  accepts. The filename matches the widget's `id`; `file_format` picks
  the encoding (`"json"`, `"toml"`, or `"text"`). Ideal for "I want a
  widget for X and don't want to write a fetcher".

  ```toml
  [[widget]]
  id = "weight"                  # reads store/weight.json
  fetcher = "basic_read_store"
  file_format = "json"
  render = { type = "chart_sparkline" }   # pins the shape to NumberSeries
  ```

  See [ReadStore](/splashboard/guides/read-store/).

The rule of thumb:

- Needs curated UX (auth, rate limits, stateful update) → built-in
  fetcher PR.
- "A number I can cron to a file" → `basic_read_store`.
- A literal string → `basic_static`.

## Adding a fetcher

The recipe lives in `src/fetcher/` — the shortest path is to copy an
existing family module (`clock/`, `system/`, `github/`) and adapt.
High-level steps:

1. Add a new file (or module) under `src/fetcher/` with your struct.
2. Implement `Fetcher` (async) or `RealtimeFetcher` (sync).
3. Declare `shapes()` honestly — every shape your fetcher may emit,
   not an aspirational list.
4. Pick a `Safety` class. Err on the side of `Network` if config can
   steer the destination.
5. Register in `Registry::with_builtins()`.
6. Add a sample row to the catalog issue (#62–#68) so the roadmap
   stays honest.
7. Run `cargo xtask` — the reference page under
   `/reference/fetchers/<family>/<name>/` generates from your impl.

The test harness already checks every fetcher is registered,
round-trips its sample body through its declared shapes, and renders
via its default renderer without panicking. Add fetcher-specific
tests in a `#[cfg(test)]` block next to the impl.
