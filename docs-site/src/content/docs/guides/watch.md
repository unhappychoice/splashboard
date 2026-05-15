---
title: Watch mode
description: Run splashboard as a persistent foreground dashboard instead of a one-shot splash.
---

The default splash paints once on shell startup or `cd` and hands the
terminal straight back to your prompt. `splashboard watch` does the opposite:
it holds the alternate screen and keeps the dashboard live until you exit.

## When to use it

- You want a glance-able dashboard on a second monitor or tmux pane.
- You're tuning a config and want to see the result update as you save.
- You want to watch CI / weather / heatmaps refresh in real time.

The shell-hook splash stays splashboard's identity — `watch` is a
deliberate foreground invocation, not the default. It is not the
splash; it's a different mode that reuses the same widgets.

## Running it

```bash
splashboard watch
```

The config is resolved exactly like the splash: a project-local
`./.splashboard/dashboard.toml` (walked up from `cwd`) wins; otherwise
the home / project dashboard under `$HOME/.splashboard/` is used. Unlike
the shell-hook splash, `watch` ignores the `auto_home` / `auto_on_cd`
opt-outs — you typed the command, so it runs.

`q` or Ctrl-C exits. The alternate screen is restored on clean exit,
panic, or `?`-bubbled errors via a `Drop` guard plus a panic hook.

## What's live

Inside the watch loop:

- **Realtime fetchers** (`clock`, `system_*`, `clock_*`) are recomputed
  on a ~200 ms throttle. The seconds digit on a `clock` widget ticks
  once per second; CPU / memory gauges fluctuate as you load the system.
- **Cached fetchers** (`git_*`, `github_*`, `rss`, `*_watchlist`, …)
  are refreshed on their own TTL by an in-process fetch loop, not via
  the detached daemon the splash uses. The foreground reads only from
  memory; disk is touched once at the top (warm-start) and once at the
  bottom (best-effort flush).
- **Animated renderers** play once during the 2-second startup window
  and then sit static. They do not replay on data updates — the
  one-shot animation contract still applies. The data-update channel is
  the cached / realtime refresh itself, not a re-played reveal.
- **Redraw-on-change**: an idle dashboard does not redraw at the frame
  rate. A repaint happens only when a payload changed, a cache TTL
  expired, the terminal resized, or the footer countdown advanced — so
  watch idles at roughly 1–5 redraws / second.

## Status footer

The bottom row is reserved for chrome:

```
 ⟳ next refresh in 45s                              q / Ctrl-C  quit
```

The left segment is a priority status line:

| When | The footer shows |
|---|---|
| Any cached widget still loading | `loading N widgets…` |
| Anything stale right now | `refreshing…` |
| Steady state with cached widgets | `next refresh in <duration>` (the soonest TTL expiry) |
| Realtime-only dashboard | `realtime` |

The right segment lists the active key bindings. The whole footer is
painted on the theme's `bg_subtle` surface so it reads as chrome
separate from the dashboard band.

## Trust still applies

The trust model (see [Trust model](/guides/trust/)) is unchanged: a
project-local dashboard that hasn't been
[`splashboard trust`](/guides/trust/#granting-trust)-ed still renders
Network widgets as `🔒 requires trust` placeholders. The gate is
evaluated once at startup, same as the splash.

## Cache lifecycle

`watch` is single-process, so the disk cache that exists for splash ↔
daemon IPC is unnecessary on the hot path. The watch loop:

1. **Boots warm**: existing entries for the configured widgets are
   copied from `$HOME/.splashboard/cache/` into an in-memory map.
2. **Runs in-memory**: every read and write during the session is a
   HashMap operation. No disk I/O on the hot path; no `.lock` files
   created.
3. **Flushes on clean exit**: the in-memory state is written back so
   the next splash or watch warm-starts from the latest values. A
   `SIGKILL` skips the flush, but the next session simply refetches on
   TTL — nothing breaks.

## Limits

- `watch` is not wired into the shell hook. It is a deliberate
  foreground command; do not source it from your rc.
- `watch` is not an idle screensaver loop. The animation contract from
  the one-shot splash still holds: animations play once, then settle.
- Concurrent `splashboard` invocations (a splash running while `watch`
  is up) read the disk cache as it was at the last flush. The in-memory
  state is not visible across processes.
