---
title: Trust model
description: Per-widget consent for Network widgets in project-local dashboards.
---

Per-directory dashboards mean that `cd`-ing into a cloned repo could
render a splash before you've read its config. splashboard bounds that
blast radius by classifying every fetcher and gating the risky ones
behind an explicit consent step.

## Safety classes

Each fetcher declares a class at compile time:

| class | examples | runs from an untrusted local config? |
|---|---|---|
| **Safe** | `clock`, `git_*`, `system_*`, `github_*` (host is hardcoded, token only leaves to `api.github.com`) | Yes, always. Safe widgets render regardless of trust state. |
| **Network** | anything whose config can steer traffic to an arbitrary host (RSS / calendar / user-URL fetchers) | Only after `splashboard trust`. Otherwise replaced with a `🔒 requires trust` placeholder. |

The classification is **per-fetcher, not per-file**. A local dashboard
that mixes a Safe `git_status` with a Network `rss` fetcher renders the
`git_status` immediately and gates the `rss` behind trust. You see what's
local right away; the external call waits for consent.

Every fetcher that ships today is classified `Safe`, so you won't see
the consent prompt yet. The gate described below is already active — it
kicks in the moment a Network widget (RSS, calendar feed, custom URL)
lands in a local config.

### What counts as "fixed host"?

A `github_*` fetcher talks to `api.github.com` — the URL is hardcoded in
the fetcher struct, config can't redirect the token somewhere else.
That's Safe even though there's authentication involved: the credential
never leaves the known host.

A hypothetical `http_fetch` that takes a user-supplied URL is
**Network**, even with no credentials: the config controls where
traffic goes, which is the whole threat.

## Which configs are trust-gated?

Trust applies only to **project-local** dashboards
(`./.splashboard/dashboard.toml` or `./.splashboard.toml`). These are the
files that travel with a cloned repo.

Implicitly trusted (no consent step):

- `$HOME/.splashboard/settings.toml`
- `$HOME/.splashboard/home.dashboard.toml`
- `$HOME/.splashboard/project.dashboard.toml`
- The baked-in fallbacks when those files are absent.

You own HOME — anything you put there is authoritative.

## The trust flow

```bash
# From anywhere inside the repo — walks up to find the nearest local dashboard.
splashboard trust

# Explicit path
splashboard trust ./path/to/.splashboard.toml

# Revoke
splashboard revoke

# List everything currently trusted
splashboard list-trusted
```

`splashboard trust` prints a capability diff (which Network widgets the
config wants to run) and prompts `y/N` before recording consent.

Trust is stored as `(canonical_path, sha256)` in
`$HOME/.splashboard/trust.toml`. Editing the dashboard file changes its
hash, which **automatically revokes trust** — the next render shows the
placeholder until you re-run `splashboard trust`.

## `SPLASHBOARD_TRUST_ALL`

```bash
export SPLASHBOARD_TRUST_ALL=1
```

Bypasses the gate for every config. Intended for CI / scripted setups
where the consent prompt can't run. Do not set this in your default
shell — it defeats the whole point of the classification.

## Why not just "trust the whole directory"?

Because the threat isn't the directory, it's **one specific widget**.
Trusting the whole file means a cloned repo that later adds a Network
widget would run it without re-prompting. Per-widget classification with
a hash-keyed trust entry catches that: any change to the config body
re-invalidates trust.

## What's not in the model

The classification has a third class, `Exec`, reserved for subprocess
widgets. It's **permanently unreachable** — subprocess plugins and
`command = "..."` widgets were closed by design. splashboard is a
curated renderer, not a shell script host. Custom widgets ship as
built-in PRs or use the
[ReadStore](/guides/read-store/).
