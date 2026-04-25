---
name: add-fetcher-renderer
description: Implement a new fetcher or renderer for splashboard. Use when the user asks to add a fetcher (e.g. `system_battery`, `weather_*`, `git_*`) or a renderer (e.g. `gauge_battery`, `chart_radar`). Drives the workflow: spec-out → code → self-review → user smoke-tests via `.splashboard/dashboard.toml` → wrap up.
---

# add-fetcher-renderer

Implements one fetcher *or* one renderer (not the whole `Fetcher × Renderer × Layout` composition — the layout slot is the user's call). Read `AGENTS.md` first for the architectural contracts; this skill only covers process.

## Phase 1 — Spec-out (要件確認)

**Don't write code yet.** Confirm the shape of the work in conversation:

1. **Fetcher or renderer?** If both, do them as two passes through this skill, not one.
2. **Catalog target.** Cross-check `#41` and the relevant sub-issue (`#61` renderers, `#62`–`#68` fetchers). If the candidate is in a checkbox list, link it. If it isn't, ask whether to add it.
3. **For fetchers:**
   - **Family prefix** (`clock_*`, `system_*`, `git_*`, …) — pick the right one or justify a new family.
   - **Cached or realtime?** Realtime contract is `< 1ms, infallible, no I/O`. Anything that touches `reqwest` / disk / process state is cached, not realtime.
   - **Safety class.** Ask "can config control where the traffic goes?" If yes → `Network` (rss / calendar / any fetcher with a `url` option). If no, even an authenticated fixed-host fetcher is `Safe`. `Exec` is closed (#5 / #20 — don't reintroduce).
   - **Output shape(s).** A multi-shape fetcher branches on `ctx.shape` inside `compute` / `fetch` and lists every variant in `shapes()`.
   - **Options.** Per-fetcher options struct (`#[derive(Deserialize)]`, `serde(deny_unknown_fields)`) parsed from `ctx.options`; surface each field as an `OptionSchema` entry so the docs generator picks it up.
4. **For renderers:**
   - **Family prefix** (`text_*`, `list_*`, `chart_*`, `gauge_*`, `grid_*`, `status_*`, `media_*`, `animated_*`). Module name = registered name.
   - **Accepted shapes.** Usually one. Multi-shape only if the visual treatment honestly fits all of them.
   - **`animates()`** — return `true` only if output changes across calls within a single draw cycle.
   - **Options.** First check whether an existing field on `RenderOptions` already covers it (`align`, `color`, `label`, `marker`, `bullet`, `style`, …). Add a new field only when none fits, and wire an `OptionSchema` entry on the renderer.
5. **Color semantics warning** (renderers). Don't bake a context-specific colour map as the default. The `gauge_battery` lesson: a battery-coloured fill (low → red) silently mis-coloured every "fraction used" widget. If the colour mapping depends on caller intent, default to `theme.text` and add an opt-in `tone` / `mode` option.
6. **Memory check.** If the user says 考えたい / 検討したい, propose options and stop — don't branch yet.

State the plan back in 3–5 bullets and wait for confirmation before Phase 2.

## Phase 2 — Implement (実装)

1. **Files.**
   - Fetcher: `src/fetcher/<family>/<name>.rs` (or `src/fetcher/<family>.rs` if the family is one file).
   - Renderer: `src/render/<family>_<variant>.rs`.
2. **Pattern.** Open the closest sibling and mirror its structure (constants → struct → trait impl → helpers → tests).
3. **Wire up.**
   - Renderer: `mod` line + `r.register(...)` in `src/render/mod.rs::Registry::with_builtins`, add the name to the `registry_resolves_all_builtins` test list. If it becomes the default for a shape, update `default_renderer_for`. Override `xtask/src/snapshots.rs::renderer_dimensions` only when the default `(40, 5)` doesn't fit the renderer's natural aspect.
   - Fetcher: add to `realtime_fetchers()` / `cached_fetchers()` in the family module's registration entry point.
4. **New shape?** Five-touch-point change per AGENTS.md: `Body` variant + `*Data` struct, `Shape` enum, `shape_of()`, `default_renderer_for()`, at least one renderer's `accepts()`.
5. **Tests.** Inline `#[cfg(test)] mod tests`.
   - Renderer: use `src/render/test_utils.rs` (`render_to_buffer_with_spec`, `line_text`). Cover each visual mode (compact vs boxed, with/without label, clamp out-of-range, narrow area).
   - Fetcher: cover each shape branch + options parsing. Realtime fetchers test `compute` directly; cached fetchers test `sample_body` and the actual fetch path with whatever stand-in is appropriate.
6. **House style.** English in code/comments/commits. `map` / `filter` / `for_each` over imperative loops. Functions ~10–15 lines, files ~100 lines as a guideline. Minimal comments — only when the *why* is non-obvious.

## Phase 3 — Self-review (各種レビュー)

Run the gates **before** asking the user to verify:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Then read your own code critically. Common findings worth catching yourself:

- **Naming**: family prefix consistent? module name matches registered name?
- **Edge cases**: empty body, `width == 0`, very narrow area, ratio out of `[0,1]`, fetchers handling timezone / locale defaults.
- **Color semantics** (renderer): does the default mapping make sense for every fetcher listed under `Compatible fetchers`? Add a `tone`-style opt-in if not.
- **Safety class** (fetcher): re-apply the Phase 1 rule — config-controlled destination → `Network`, otherwise `Safe`.
- **Test coverage**: each visual mode / shape branch has a test? `clamps_out_of_range`, `empty_area_does_not_panic`, narrow-area degradation if applicable.
- **Catalog parity**: `cargo run --bin splashboard -- catalog renderer <name>` (or `... fetcher <name>`) prints the option schema and compatible counterparts you intended.

Report findings as a punch list with severity (🔴 / 🟡 / 🟢). Apply 🔴 fixes before Phase 4.

## Phase 4 — User smoke-test (動作確認)

The CLI requires a real tty, so you can't visually verify colours yourself. Hand off to the user:

1. **Write a temporary `./.splashboard/dashboard.toml`** exercising the new fetcher / renderer in 2–3 configurations (renderer: default + each opt-in tone / variant + compact size; fetcher: with and without each meaningful option).
2. **Tell the user:** `cargo run --bin splashboard` from the repo root.
3. **Wait for confirmation.** Visual / colour issues that ASCII tests miss show up here.

## Phase 5 — Wrap up (完了)

- **Delete `.splashboard/dashboard.toml`** after the user signs off — it was a throwaway smoke-test fixture, not a ship artifact. Keep it only when the user explicitly says so. Note: in splashboard's own repo a committed `.splashboard/dashboard.toml` doubles as the "ship-with-the-clone" project dashboard, so committing one is a separate decision the user has to make deliberately, not a side-effect of the smoke-test.
- **Don't commit unprompted.** Wait for explicit ask. When you do commit:
  - Conventional commit (`feat(render): ...`, `feat(fetcher): ...`).
  - `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer.
  - Tick the relevant `#41` sub-issue checkbox in the PR description, not in code.
- **PR description** mirrors the auto-generated reference page so reviewers see what `/reference/...` will show after the docs site rebuilds. Run `cargo xtask` locally; copy the relevant block from `docs-site/src/content/docs/reference/{renderers,fetchers}/<family>/<name>.md` (untracked — `.gitignore`'d). Strip the frontmatter, keep `Accepts` / `Animates` / `Options` / `Theme tokens` / `Compatible fetchers` (or `Compatible renderers` for fetchers), and rewrite the relative `../../...md` links to absolute `https://splashboard.unhappychoice.com/reference/...` URLs.
- **Merge strategy** is `--merge` (merge commit). Never rebase or squash — captured project preference.

## Out of scope

- **Plugin protocol (#5)** and **command widget (#20)** are closed. If the user's request shape is "run a subprocess and render its output", explain why it's rejected and steer toward `basic_read_store` (the user writes the file) or a dedicated built-in.
- **Screensaver-style persistent loops.** Animation lives within the `ANIMATION_WINDOW`; a dedicated idle-loop sub-mode is a different product.
