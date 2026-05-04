# Use case dashboards

Sample home / project dashboards that ship as inspiration on the
[Use cases page](https://splashboard.unhappychoice.com/showcases/usecases/) but
aren't preset-quality — they assume a specific city, account, RSS feed, or
other environment-specific bit, so they wouldn't render well for everyone out
of the box. Templates in `src/templates/` carry the bar of "works on a fresh
install"; this directory is everything one curation tier below.

For user-submitted dashboards (someone else's setup shared as inspiration), see
`examples/community/` instead — the curation tier is even looser there.

## Adding a use case

Drop a single `*.toml` file in this directory with a `[showcase]` table at the
top, followed by the regular `[[widget]]` / `[[row]]` config:

```toml
[showcase]
title = "Tokyo morning briefing"
description = "JST clock, neighborhood weather, Hacker News scroll for the train ride."
context = "home"        # "home" or "project"
requires = ["weather (network)", "Hacker News (network)"]

[[widget]]
id = "..."
fetcher = "..."
# ...
```

`cargo xtask` renders the dashboard to
`docs-site/src/assets/rendered/usecases/<slug>.html` using each fetcher's
`sample_body`, and writes a metadata index that the use cases page reads to
build the gallery. No network or filesystem access at build time.

The filename (without `.toml`) becomes the slug. Convention: `home_*.toml`
for home dashboards, `project_*.toml` for project dashboards.

## What goes here vs `src/templates/` vs `examples/community/`

| | `src/templates/` | `examples/usecases/` | `examples/community/` |
|---|---|---|---|
| Bundled in the binary | yes | no | no |
| `splashboard install --template` | yes | no | no |
| Renders well on a fresh install | yes | no | no |
| Curation bar | tight | medium | loose |
| Authored by | maintainers | maintainers | users (PR submissions) |

If a use case ages into something universally useful, promote it to a template.
If a community submission is broadly applicable, promote it to a use case.
