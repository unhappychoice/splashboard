# splashboard

[![CI](https://github.com/unhappychoice/splashboard/actions/workflows/ci.yml/badge.svg)](https://github.com/unhappychoice/splashboard/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/unhappychoice/splashboard/branch/main/graph/badge.svg)](https://codecov.io/gh/unhappychoice/splashboard)

A customizable terminal splash rendered on shell startup and on `cd`.

> `splashboard` = `splash` + `dashboard` — a splash screen for your shell, rendered as a dashboard.

## What is this

Every time you open a terminal, you see... a blinking cursor. What if you saw a dashboard of the things you actually care about — greetings, git status, CI health, PRs awaiting review, a GitHub contribution heatmap, the moon phase?

`splashboard` renders a customizable TUI splash from a TOML config, cached-first so it's instant, refreshed in the background so it stays current. One line in your `.bashrc` / `.zshrc` / `config.fish` and you're done.

## Killer feature: per-directory splashes

`cd` into a repo that ships a `.splashboard/config.toml` and the splash reshapes itself to that project — its CI, its branch, its PRs — without you configuring anything. Walk-up discovery starts from CWD, so it works from anywhere inside the tree. Competitor products (neofetch, fastfetch, starship) don't do this.

Three first-class contexts:

| context | value axis | config source |
|---|---|---|
| self / home | daily delight, ambient info | `$HOME/.splashboard/config.toml` |
| self / project | operational (CI, branch, PRs) | `./.splashboard/config.toml` (per-dir, walk-up) |
| other / project | craft + wow for cloners | `./.splashboard/config.toml` shipped with the repo |

## Install

```bash
cargo install splashboard
splashboard install
```

`splashboard install` detects your shell, walks through four previewed pickers — home
template, project template, theme, and a toggle screen for `bg` / `wait_for_fresh` — shows
a final confirmation page with the resolved plan, then writes `home.dashboard.toml` +
`project.dashboard.toml` + a starter `settings.toml` to `$HOME/.splashboard/` and wires
your shell rc. Re-running is idempotent: files whose content is still current stay
untouched, and anything that changes lands the prior copy in a `.bak` sidecar first.
Non-interactive flow for dotfiles bootstrap:

```bash
splashboard install \
  --shell zsh \
  --home-template home_splash \
  --project-template project_github \
  --theme tokyo_night \
  --no-bg \
  --wait
```

Prefer to own the rc edit yourself? Append one line that re-sources `splashboard init <shell>` on every shell start — that's the same line `splashboard install` writes, and it means upgrades to splashboard ship an updated init snippet automatically:

```bash
echo 'eval "$(splashboard init zsh)"'                              >> ~/.zshrc
echo 'eval "$(splashboard init bash)"'                             >> ~/.bashrc
echo 'splashboard init fish | source'                              >> ~/.config/fish/config.fish
echo 'Invoke-Expression (& splashboard init powershell | Out-String)' >> $PROFILE
```

The init snippet renders on new shells and re-renders when you `cd` into a directory that holds a project-local dashboard.

## Configuring a splash

A **widget** is the composition of a **fetcher** (what data), a **renderer** (how to draw it), and a **layout slot** (where on the grid).

```toml
# $HOME/.splashboard/config.toml

[[widget]]
id = "clock"
fetcher = "clock"
render = { type = "text_ascii", pixel_size = "quadrant", align = "center" }

[[widget]]
id = "stars"
fetcher = "github_repo_stars"
render = { type = "text_plain", align = "center" }

[[widget]]
id = "commits"
fetcher = "git_commits_activity"
render = "chart_sparkline"

[[row]]
height = { length = 4 }
  [[row.child]]
  widget = "clock"

[[row]]
height = { length = 3 }
  [[row.child]]
  widget = "stars"
  [[row.child]]
  widget = "commits"
  title = "commits/day"
  border = "rounded"
```

Same fetcher can drive multiple renderers — `clock` renders as `text_ascii`, `text_plain`, or `animated_typewriter`, whichever fits the row.

## What's built in

### Fetchers

- **basic_*** — `basic_static` (literal text / text blocks), `basic_read_store` (deserializes `$HOME/.splashboard/store/<id>.<ext>` into any supported shape — the escape hatch for "I want a custom widget")
- **clock_*** — `clock`, `clock_timezones`, `clock_ratio`, `clock_state`, `clock_derived`, `clock_sunrise`, `clock_countdown`
- **system_*** — `system`, `system_cpu`, `system_memory`, `system_load`, `system_uptime`, `system_processes`, `system_disk_usage`
- **git_*** — `git_status`, `git_recent_commits`, `git_commits_activity`, `git_contributors`, `git_blame_heatmap`, `git_stash_count`, `git_worktrees`, `git_latest_tag`
- **github_*** — action status/history, PRs (mine / review-requested / repo), issues (assigned / repo / good-first), releases, notifications, stars, contributions heatmap, contributors

### Renderers

Names follow a `family_variant` convention so siblings sort together:

- **text_*** — `text_plain`, `text_ascii`, `animated_typewriter`
- **list_*** — `list_plain`, `list_timeline`
- **grid_*** — `grid_table`, `grid_calendar`, `grid_heatmap`
- **gauge_*** — `gauge_circle`, `gauge_line`
- **chart_*** — `chart_sparkline`, `chart_line`, `chart_scatter`, `chart_bar`, `chart_pie`
- **status_*** — `status_badge`
- **media_*** — `media_image`

### Browse the catalog

```bash
splashboard catalog                   # overview
splashboard catalog fetcher           # list fetchers
splashboard catalog fetcher git_status  # options + compatible renderers
splashboard catalog renderer grid_heatmap  # options + compatible shapes
```

Or browse the generated reference at <https://unhappychoice.github.io/splashboard/>.

## How fast rendering works

Startup never blocks the shell. Two fetcher flavors split the work:

- **Cached (async)** — disk cache with TTL; the splash reads from cache and paints immediately, then a detached child refreshes in the background for next time. Right for anything that does I/O.
- **Realtime (sync, per-frame)** — recomputed on every draw tick, no cache. Right for "right now" values (`clock`, `system_cpu`, `system_uptime`, `clock_countdown`). Contract: < 1ms, infallible, no I/O.

Use `--wait` if you'd rather block for fresh data than paint stale.

## Filesystem layout

All splashboard state lives under **`$HOME/.splashboard/`** (same on Linux, macOS, Windows — no XDG paths):

```
$HOME/.splashboard/
├── config.toml        # global config
├── trust.toml         # trust store (path + sha256 entries)
├── cache/             # per-widget cache (key.json + key.lock)
└── store/             # ReadStore files — $HOME/.splashboard/store/<id>.<ext>
```

Override with `SPLASHBOARD_HOME` (tests, CI, relocatable installs). Per-directory configs stay in the repo as `./.splashboard/config.toml` or `./.splashboard.toml`.

## Trust model

Per-directory configs mean that `cd`-ing into a cloned repo could render a splash before you've read its config. To bound the blast radius, each fetcher is classified:

| class | examples | runs from an untrusted local config? |
|---|---|---|
| **Safe** — local-only reads or fixed-host authenticated network | clock, git_status, system, github_* (host is hardcoded) | yes, always |
| **Network** — URL or query is user-provided | anything whose config can steer traffic to an arbitrary host | only after `splashboard trust` |

```bash
splashboard trust         # trust the nearest .splashboard.toml (prints capability diff, prompts y/N)
splashboard revoke        # revert
splashboard list-trusted  # show all trusted configs
```

Trust is keyed by `(canonical_path, sha256)` — editing the file revokes trust automatically. Global config (`$HOME/.splashboard/config.toml`) and the baked-in default are implicitly trusted. Escape hatch: `SPLASHBOARD_TRUST_ALL=1` (documented as insecure).

Subprocess plugins and `command = "..."` widgets are [deliberately out of scope](AGENTS.md#rejected-designs-dont-reintroduce) — splashboard is a curated renderer, not a dashboard framework.

## Opt-out

The splash skips rendering when any of these is true: `stdout` isn't a terminal, `TERM=dumb`, or one of `CI` / `SPLASHBOARD_SILENT` / `NO_SPLASHBOARD` is set. Also skipped below 40×16.

## Status

Usable day-to-day. Widget catalog tracked as a living roadmap in [issue #41](https://github.com/unhappychoice/splashboard/issues/41) — new fetchers and renderers land as PRs that tick the checkboxes.

## License

ISC

## Related

- [gitlogue](https://github.com/unhappychoice/gitlogue) — cinematic git history replay
- [gittype](https://github.com/unhappychoice/gittype) — CLI typing game from your source code
- [mdts](https://github.com/unhappychoice/mdts) — local Markdown tree server
