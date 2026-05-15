---
title: Performance
description: How the cached-first splash stays fast, how the disk cache is refreshed, and how to inspect or invalidate it.
---

splashboard runs from your shell rc on every shell startup and every `cd`.
The budget is "fast enough that you don't notice"; the implementation is a
two-process model with an on-disk cache between the splash and a detached
refresher.

## Why the splash paints instantly

The hot path is intentionally tiny:

```
shell hook ──▶ splashboard
                  │
                  ▼
              read $HOME/.splashboard/cache/<key>.json (one file per cached widget)
              compute realtime widgets in process
              draw once
                  │
                  ▼
              spawn detached `splashboard fetch-only`
              hand the terminal back to the shell
```

The render-and-exit pass never blocks on network, git, or any other I/O. It
paints whatever the cache already has — even if some entries are an hour
old, even if a few are missing (a loading placeholder fills the slot). The
detached child does the actual work; the *next* splash reads its output.

This is the cached-first model. With `[general] wait_for_fresh = true` (or
`splashboard --wait`), the parent blocks on the child instead, up to a 5 s
hard deadline. You trade the snappy hand-back for fresh data on every paint.

| Mode | First paint | Fresh data |
|---|---|---|
| Default (cached-first) | Disk read of N cache files, ~immediate | On the next splash |
| `--wait` / `wait_for_fresh = true` | Up to 5 s | On this paint |

## The refresh flow

The detached child is `splashboard fetch-only` running through the same
fetcher registry as the splash. It:

1. **Re-resolves the same dashboard** the parent did (passed via `--kind` /
   `--path` so it doesn't drift if CWD changed between parent spawn and
   daemon start).
2. **Acquires a per-widget lock**: tries to create
   `$HOME/.splashboard/cache/<key>.lock` exclusively (`O_CREATE | O_EXCL`,
   PID written inside). If another in-flight fetch holds it, this widget is
   skipped — the other process will write the result. If the lock file is
   older than 30 seconds, it's treated as orphaned by a crashed sibling and
   stolen.
3. **Runs `fetch(ctx)`** with a per-widget timeout.
4. **Writes `<key>.json` atomically** (tmp file + `rename`) and drops the
   lock.

The lock is the refresh-coalescing primitive. Two concurrent shell startups
can both spawn a refresh; only one of them actually fetches each widget. The
other reads the result on the splash after.

### Stale-while-revalidate

The splash paints whatever cached entry exists, regardless of whether its
TTL has expired. A stale entry isn't suppressed — it's the data you saw
last time, which is almost always what you want for an ambient splash. An
expired entry just signals "this widget will be refreshed when the next
`fetch-only` runs".

A widget with **no cache entry yet** (first run, after `cache clear`, or
after a config change that invalidated the cache key) shows a loading
placeholder while the background fetcher runs. The splash holds for up to 2
seconds when a single widget is missing, or up to 5 seconds when the whole
cache is cold (no entry for any cached widget). If the data lands inside
that window it paints; otherwise the placeholder stays for the rest of the
splash, and the entry shows up on the next paint.

### Failure semantics

Fetcher errors and timeouts are cached too — that's what `CacheEntryKind` is
for. Three outcomes:

| Kind | When | TTL stored | Body cached |
|---|---|---|---|
| `ok` | `fetch` returned `Ok(payload)` | the widget's configured TTL | the payload |
| `err` | `fetch` returned `Err(_)` | 30 s | a `⚠ <error>` placeholder |
| `timeout` | `fetch` didn't return inside the per-fetch deadline | 30 s | a `⏱ timed out` placeholder |

Errors have a 30 s mini-TTL so a flaky backend retries soon, but a
consistently failing widget doesn't hammer it on every splash. The error
shows in-band — in the widget's slot — so users can see what's misconfigured
without crashing the splash.

## Inspecting and clearing the cache

splashboard ships a `cache` subcommand for working with the on-disk state:

```bash
splashboard cache path                # print the resolved cache directory
splashboard cache list                # one row per entry: key, age, TTL, fresh, kind, size
splashboard cache list --json         # same data as JSON for scripting
splashboard cache clear               # wipe every cache entry (with confirmation)
splashboard cache clear --yes         # skip the prompt
splashboard cache clear <widget-id>   # remove only one widget's entry
```

The single-widget form computes the cache key from whichever dashboard would
render in the current directory. If you're standing in a project whose
dashboard doesn't contain that widget, `cd` to a matching directory first,
or use `cache list` plus a manual delete by key.

The default cache key is `<fetcher>-<digest>`, where the digest is the
leading 8 bytes of `sha256("<name>|<shape>|<format>")` rendered in hex.
Fetchers whose output depends on more (`cwd`, repo URL, options) override
`cache_key()` to fold those in. That's why two `clock` widgets with the
same `format` (and therefore the same key) share a payload — one fetch
refreshes both.

## Tuning

Three knobs cover most use cases:

- **Per-widget `refresh_interval = N`** — TTL for that widget's cache, in
  seconds. Lower = fresher on each splash, higher = less background work.
  Ignored by realtime fetchers (`clock`, `system_*`).
- **`[general] wait_for_fresh = true`** — trade snappy first paint for
  guaranteed-fresh data. Useful for kiosk dashboards or when you want a
  scripted `splashboard` invocation to always see the latest values. The
  wait is capped at 5 s; widgets that miss the deadline still get refreshed
  by the daemon and show up on the next splash.
- **`splashboard cache clear`** — bust the cache after a config change you
  want to verify immediately, or to recover from a corrupted entry.

## Realtime budget

Realtime fetchers (`clock`, `clock_*`, `system_*`) run synchronously
without I/O. The contract is
[documented in concepts/fetcher](/guides/concepts/fetcher/#the-realtime-contract):
**< 1 ms, infallible, no I/O.**

How often they actually run depends on mode:

- **Splash (one-shot)** — computed once at startup and painted, even when
  the multi-frame animation / wait loop is active. The budget exists so the
  single paint isn't delayed; the loop only re-reads the cache and ticks
  animated renderers, it does not recompute realtime payloads.
- **`splashboard watch`** — recomputed on a ~200 ms throttle inside the
  foreground loop. The seconds digit on a `clock` ticks once per second;
  `system_*` gauges fluctuate as you load the host.

If you want HTTP / git / filesystem in a "realtime-feeling" widget, write a
cached fetcher with a short `refresh_interval` instead — the splash will
pick up the new value on the next paint.

## Watch mode is different

[`splashboard watch`](/guides/watch/) swaps the two-process disk-IPC model
for an in-process in-memory cache: a single process holds the dashboard live
on the alternate screen, refreshes cached widgets through an in-process
fetch loop, and only touches the disk cache to warm-start at the top and
best-effort-flush at the bottom. See
[Cache lifecycle](/guides/watch/#cache-lifecycle) for the details.
