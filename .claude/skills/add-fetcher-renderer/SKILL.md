---
name: add-fetcher-renderer
description: Implement a new fetcher or renderer for splashboard. Use when the user asks to add a fetcher (e.g. `system_battery`, `weather_*`, `git_*`) or a renderer (e.g. `gauge_battery`, `chart_radar`). Drives the workflow: pick-candidate → spec-out → code → self-review → user smoke-tests via `.splashboard/dashboard.toml` → wrap up.
---

# add-fetcher-renderer

Implements one fetcher *or* one renderer (not the whole `Fetcher × Renderer × Layout` composition — the layout slot is the user's call). Read `AGENTS.md` first for the architectural contracts; this skill only covers process.

**Phases 0 and 1 are blocking on a user reply.** Auto mode does *not* license you to pick a candidate or finalise a spec on the user's behalf — both decisions ship in user-visible config (`render = "..."` / `fetcher = "..."`) and are expensive to rename. Always ask a concrete question and stop until the user answers. Only Phase 2 onward proceeds without further prompts.

## Phase 0 — Pick the candidate (候補選定)

Skip this phase only if the user named a specific candidate ("add `system_battery`", "implement `gauge_segment`"). Otherwise — including any open-ended ask ("add some renderer", "implement a fetcher") — work through this phase before Phase 1.

1. **Survey ALL relevant catalog issues in parallel.** Don't pick from memory; the boxes churn — yesterday's open box may already be checked.
   - Renderer ask → `#61` only.
   - Fetcher ask → **all of `#62` through `#68`**, dispatched as a single batch of parallel `gh issue view` calls. No partial surveys "starting with Local/System and seeing what fits". The biggest daily-driver classes (task tools in `#66`, coding-time in `#64`, music in `#67`) sit exactly where a partial survey would stop early.
   - Cross-cutting context (composition model, ReadStore recipes, rejected designs): `gh issue view 41 --json body -q .body`.
2. **Filter to the open boxes** in the surveyed issues.
3. **Score candidates with these four filters** (these supersede #41's looser rubric for ranking — keep #41 for cross-cutting context, do scoring here):

   **A. Renderer primitive leverage (renderer asks).** Counts only if you can name **≥2 specific consumer fetchers** from the current catalog — shipped or in an open box, by name. Vague "could be useful for some Ratio fetcher" doesn't count. `text_big_number` clears it (github_repo_stars, oss_followers_delta, deploy_error_budget, package downloads). `chart_funnel` doesn't (no roadmap fetcher emits funnel-shaped Bars yet) — defer until a consumer lands.

   **B. Recurring-change × repeated-look-value** (the real "daily-driver" test, replacing #41's vague rubric ②). Both sub-tests must pass for a high rank:
   - *Recurring-change*: does displayed content change ≥1×/day?
   - *Repeated-look-value*: would the user still want to see it after 100 splash opens? **Novelty traps fail here** — `iss_position` (fun once, ignored after), `f1_next_race` (fan-only then forgotten), `random_cat`, `fortune`. `todoist_today` / `linear_assigned` / `slack_unread` pass — people *check these on purpose every day*.

   **C. Persona name test.** Name in one phrase the user who'd open splash and benefit **every day**. "Anyone with a Todoist account" ✅. "Devs at modern startups using Linear" ✅. "Kids learning about space" ❌ (not a regular shell user). "People who like weather data" ❌ (they check phones). If you can't name a concrete recurring user, demote to ambient/novelty tier.

   **D. Setup-cost is an amortized tiebreaker, not a gate.** Distinguish:
   - *Zero-config* (drop-in TOML works) → no penalty.
   - *One-time setup* (token / lat-lon / country code) → amortized over years of daily glances. Negligible penalty when B+C are strong. `todoist_today`'s token is paid back in week 1 of daily use.
   - *Per-use friction* (OAuth re-auth, custom URL each invocation) → real penalty.

   The old "Network class with required setup loses to a Safe sibling" rule was too coarse. A `Safe` widget no one looks at loses to a `Network` widget actively checked daily. Gate on whether the persona will pay the setup cost, not on Safety class itself.

4. **Present a 3–5 candidate shortlist**, one line each: catalog box being ticked, family, and **which of A/B/C/D scored it where** — not vague phrases like "fills a gap". Lead with the top pick, note a runner-up. Include candidates from at least 2 different scoring profiles (e.g., one A-strong renderer primitive, one B+C-strong daily-driver fetcher, optionally one ambient for contrast) so the user sees the real trade-off space, not just one cluster.
5. **Ask the user which to implement.** Stop. Do not start Phase 1 on a guess. Auto mode does not override this — the candidate decision is the user's, not yours.

## Phase 1 — Spec-out (要件確認)

**Don't write code yet.** Confirm the shape of the work in conversation:

1. **Fetcher or renderer?** If both, do them as two passes through this skill, not one.
2. **Catalog target.** Cross-check `#41` and the relevant sub-issue (`#61` renderers, `#62`–`#68` fetchers). If the candidate is in a checkbox list, link it. If it isn't, ask whether to add it.
3. **For fetchers:**
   - **Family prefix** (`clock_*`, `system_*`, `git_*`, …) — pick the right one or justify a new family.
   - **Cached or realtime?** Realtime contract is `< 1ms, infallible, no I/O`. Anything that touches `reqwest` / disk / process state is cached, not realtime.
   - **Safety class.** Ask "can config control where the traffic goes?" If yes → `Network` (rss / calendar / any fetcher with a `url` option). If no, even an authenticated fixed-host fetcher is `Safe`. `Exec` is closed (#5 / #20 — don't reintroduce).
   - **Output shape(s).** A multi-shape fetcher branches on `ctx.shape` inside `compute` / `fetch` and lists every variant in `shapes()`. **Enumerate every `Shape` variant from `src/render/mod.rs::Shape` and accept-or-reject each one** — don't just pick the obvious ones from the catalog blurb. The bar for *acceptance* is "can this data be meaningfully expressed in this form?", not "is this the most natural form?". A list of files renders as Text (comma-joined `"main.rs (42), lib.rs (31), …"`), TextBlock (one row each), Entries (path→count), Bars (label+value) — all four pass. Look at how the closest sibling fetcher in the same family handled each shape; family precedent is a strong signal. Reject **only** when no representation exists (Heatmap needs a 2D grid you don't have, Timeline needs chronological order, Calendar needs a year/month). "Feels redundant with another shape" or "the catalog didn't mention it" are not rejection reasons. Present the full 12-row table in the spec so the user can challenge any verdict before code lands.
   - **Options.** Per-fetcher options struct (`#[derive(Deserialize)]`, `serde(deny_unknown_fields)`) parsed from `ctx.options`; surface each field as an `OptionSchema` entry so the docs generator picks it up.
4. **For renderers:**
   - **Family prefix** (`text_*`, `list_*`, `chart_*`, `gauge_*`, `grid_*`, `status_*`, `media_*`, `animated_*`). Module name = registered name.
   - **Accepted shapes.** **Exactly one.** Primary renderers are single-shape — the only exception is animated post-process wrappers (`animated_postfx`, `animated_boot`, `animated_scanlines`, `animated_splitflap`, `animated_wave`) that delegate to an inner renderer via `ALL_SHAPES`. If two shapes feel like they should share a renderer, register two renderers that share helpers (`text_plain` vs `list_plain` is the canonical pair).
   - **`animates()`** — return `true` only if output changes across calls within a single draw cycle.
   - **Options.** First check whether an existing field on `RenderOptions` already covers it (`align`, `color`, `label`, `marker`, `bullet`, `style`, …). Add a new field only when none fits, and wire an `OptionSchema` entry on the renderer.
5. **Color semantics warning** (renderers). Don't bake a context-specific colour map as the default. The `gauge_battery` lesson: a battery-coloured fill (low → red) silently mis-coloured every "fraction used" widget. If the colour mapping depends on caller intent, default to `theme.text` and add an opt-in `tone` / `mode` option.
6. **Memory check.** If the user says 考えたい / 検討したい, propose options and stop — don't branch yet.

State the plan back in 3–5 bullets and **ask an explicit yes/no question** ("Proceed with this plan?") — don't just narrate the plan and roll into Phase 2. Stop until the user answers. Auto mode does not override this; naming, options, and safety class all leak into user-facing config and are painful to undo.

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
- **Tick the catalog checkbox after merge.** Once the PR lands, edit the relevant sub-issue (`#61` for renderers, `#62`–`#68` for fetchers) and flip the candidate's `[ ]` to `[x]`. Replace the planned blurb with a one-line "Shipped in #NNN" note plus any non-obvious behaviour decisions worth recording (auto-fit, opt-in options, threshold quirks). The PR description alone isn't enough — the issue body is the catalog index everyone scans for what's still open. Use `gh issue view <N> --json body -q .body` to dump it, edit, then `gh issue edit <N> --body-file <path>` to apply.

## Out of scope

- **Plugin protocol (#5)** and **command widget (#20)** are closed. If the user's request shape is "run a subprocess and render its output", explain why it's rejected and steer toward `basic_read_store` (the user writes the file) or a dedicated built-in.
- **Screensaver-style persistent loops.** Animation lives within the `ANIMATION_WINDOW`; a dedicated idle-loop sub-mode is a different product.
