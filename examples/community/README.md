# Community dashboards

Splashes shared by users in the wild — your splashboard config, in your words,
on the
[Community page](https://splashboard.unhappychoice.com/showcases/community/).
This is the loosest tier: no requirement that the config work in anyone else's
environment. If it makes sense in your terminal, that's enough.

## Submitting

1. Drop a single `*.toml` file in this directory with the `[showcase]` table
   below at the top, followed by the regular `[[widget]]` / `[[row]]` config.
2. Open a PR. Maintainers run `cargo xtask` and the gallery picks the entry up
   automatically — no extra wiring.

```toml
[showcase]
title = "Your dashboard's name"
description = "One sentence on what makes it yours — what you wanted to see every morning, what you cut, what you reach for."
context = "home"        # "home" or "project"
requires = []           # optional — list any external accounts / services it leans on
author = "Your Name (@github_handle)"   # optional
source = "https://github.com/you"       # optional, link to your dotfiles or a discussion

[[widget]]
id = "..."
fetcher = "..."
# ...
```

The filename (without `.toml`) becomes the slug. Convention: lowercase, prefer
`home_<handle>_<flavor>.toml` or `project_<handle>_<flavor>.toml` so multiple
submissions from the same author sort together.

## What goes here vs `examples/usecases/`

| | `examples/usecases/` | `examples/community/` |
|---|---|---|
| Authored by | maintainers | users (PR submissions) |
| Curation bar | medium — should make sense as inspiration | loose — anything that makes sense to its author |
| Promoting up | promote into `src/templates/` if universally useful | promote into `examples/usecases/` if broadly applicable |

If a community submission is broadly applicable, a maintainer may promote it to
`examples/usecases/` (with credit retained). If it works on a fresh install,
all the way up to `src/templates/`.
