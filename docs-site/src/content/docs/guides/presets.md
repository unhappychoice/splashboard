---
title: Presets
description: Four curated dashboards you can adopt verbatim — one per context.
---

splashboard 0.x ships four curated dashboard presets. Each is a composed
design (hero / subtitle / divider / body / hierarchy / breathing), not a
widget stack. Pick one as a starting point and edit freely.

Preset names follow a `context_intent` convention: `home_*` for the ambient
"new shell" splash, `project_*` for the "cd into a repo" splash.

:::note
The `home_*` presets depend on fetchers that land alongside the preset
work ([issue #81](https://github.com/unhappychoice/splashboard/issues/81)
adds `weather` and `github_avatar`). Until those ship, the TOML blocks
below render a subset and the screenshots are pending. The preset work is
tracked in [issue #77](https://github.com/unhappychoice/splashboard/issues/77).
`project_github_activity` is ready to use today — the TOML is a working
config.
:::

## `home_splash` — terminal hero + ambient info

The onboarding splash. A big ASCII hero of the terminal name, a date bar,
local weather, sunrise / sunset, and a year-progress gauge. Optimized for
"new shell, every morning" — you see it more than you see any other splash.

```
          ██╗  ██╗██╗████████╗████████╗██╗   ██╗
          ██║ ██╔╝██║╚══██╔══╝╚══██╔══╝╚██╗ ██╔╝
          █████╔╝ ██║   ██║      ██║    ╚████╔╝
          ██╔═██╗ ██║   ██║      ██║     ╚██╔╝
          ██║  ██╗██║   ██║      ██║      ██║
          ╚═╝  ╚═╝╚═╝   ╚═╝      ╚═╝      ╚═╝

              terminal · powered by macOS 14.3

      ──────  23 April 2026 · Thursday · 23:47  ──────

      Tokyo  ·  🌦 18°C cloudy  ·  🌅 06:12 ↗ 17:38
      moon: waxing gibbous
      year: ▓▓▓▓▓▓▓▓░░░░░░░░░░░░  32%
```

| role | implementation |
|---|---|
| hero | `system` (kind = terminal) → `text_ascii` figlet wrapped in `animated_postfx` (hsl_shift) |
| attribution | `static` + `system` (kind = os_version) → `text_plain` dim |
| date bar | `clock` → `text_plain`, em-dash decorated |
| ambient | `static` + `weather` + `clock_sunrise` → `text_plain` |
| moon | `static` + `clock_derived` (kind = moon) → `text_plain` |
| year progress | `clock_ratio` (period = year) → `gauge_line` (percent) |

## `home_daily` — big clock + ambient hierarchy

Ambient-first, no post-effects. Dominated by a large block clock with a
date subtitle. Below: outside-the-window info (weather / sunrise / moon)
and time progress gauges (year / month / week / day) as a uniform stack.

```
     today

        ████████╗██╗  ██╗██╗   ██╗      ██████╗  ██████╗
        ╚══██╔══╝██║  ██║██║   ██║     ╚════██╗ ╚════██╗
           ██║   ███████║██║   ██║      █████╔╝  █████╔╝
           ██║   ██╔══██║██║   ██║      ╚═══██╗  ╚═══██╗
           ██║   ██║  ██║╚██████╔╝     ██████╔╝ ██████╔╝
           ╚═╝   ╚═╝  ╚═╝ ╚═════╝      ╚═════╝  ╚═════╝

               23 April 2026  ·  Thursday

     ───  outside  ─────────────────────────────────────

          🌦 cloudy 18°C         🌅 sunrise 06:12
          💨 wind 4 m/s          🌇 sunset  17:38
          💧 humidity 67%        🌙 waxing gibbous

     ───  time  ────────────────────────────────────────

          year    ████████░░░░░░░░░░   32%
          month   ███████████████░░░   79%
          week    ██████░░░░░░░░░░░░   60%
          day     █████████████░░░░░   72%
```

| role | implementation |
|---|---|
| label | `static "today"` → `text_plain` dim |
| hero | `clock` (format `%H:%M`) → `text_ascii` (blocks, quadrant) |
| date | `clock` (format `%e %B %Y · %A`) → `text_plain` dim, center |
| outside (left) | `weather` → `grid_table` |
| outside (right) | `clock_sunrise` + `clock_derived` (moon) → `grid_table` |
| time progress | 4× `clock_ratio` (year / month / week / day) → `gauge_line` percent |

## `home_github` — contributions heatmap + avatar

For GitHub-heavy daily flow. Full-width contributions heatmap, avatar
image, my PRs / review queue side-by-side, and a recent-activity
timeline.

```
  ┌──────────┐
  │          │      @ U N H A P P Y C H O I C E
  │ [avatar] │
  │          │      github profile · member since 2013
  └──────────┘

     ═══  contributions · last year  ═══════════════════

     ░░▒▒▓▓██▓▒░░▒▓█▒░░▒▓▓█░░▒▓█▒▓░░▒▓█▒░░▒▓▓░░▒▓█▒▒▓█
     ▒▒▓▓██▓▒░░▒▓█▒▒░░░▒▓█▒▓▒░▒▓█▒░░░▒▓█▒▒▓▓░░▒▓█▒▒▓▓█

     ───  attention  ───────────────────────────────────

          my PRs                  review queue
          • #42 feat: theme       • #128 fix: render
          • #41 fix: grid         • #127 chore: bump
          • #40 chore: deps       • #126 fix: style

     ───  recent  ──────────────────────────────────────

          ● 2h ago   opened #42 feat(theme): add nord
          ● 6h ago   reviewed #38 chore: bump deps
          ● 1d ago   merged  #41 fix(render): grid align
```

| role | implementation |
|---|---|
| avatar | `github_avatar` → `media_image` (fit = contain, 14×7) |
| hero | `static` (username) → `text_ascii` figlet |
| subtitle | `static` → `text_plain` dim |
| contributions | `github_contributions` → `grid_heatmap` full width |
| attention | `github_my_prs` + `github_review_requests` → `list_plain` two columns |
| recent | `github_notifications` → `list_timeline` (relative dates) |

## `project_github_activity` — repo hero + activity grid

For "cd into a repo" — ships as `./.splashboard/dashboard.toml` in this
project. Repo name as a figlet hero, GitHub metadata subtitle, and a grid
of activity panes (commits heatmap, top contributors, open PRs / issues,
languages breakdown, recent releases).

```
  ┌──────────┐
  │          │  ███████╗██████╗ ██╗      █████╗ ███████╗██╗  ██╗
  │  [logo]  │  ██╔════╝██╔══██╗██║     ██╔══██╗██╔════╝██║  ██║
  │          │  ███████╗██████╔╝██║     ███████║███████╗███████║
  │          │  ╚════██║██╔═══╝ ██║     ██╔══██║╚════██║██╔══██║
  │          │  ███████║██║     ███████╗██║  ██║███████║██║  ██║
  └──────────┘  ╚══════╝╚═╝     ╚══════╝╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝

                unhappychoice/splashboard · ★ 1.2k · ISC

     ───  languages  ───────      ───  released  ──────

          Rust     ████████ 89%       ● v0.2.0  3d ago
          TOML     █        5%        ● v0.1.9  2w ago
          Shell    ▎        2%        ● v0.1.8  1mo ago

     ───  PRs  ─────────────       ───  issues  ───────

          • #42 feat: theme           • #78 docs: guide
          • #41 fix: grid             • #77 feat: preset
          • #40 chore: deps           • #76 meta: 0.x

     ───  commits  ─────────       ───  top contributors
```

Full TOML:

```toml
# ./.splashboard/dashboard.toml

[[widget]]
id = "hero"
fetcher = "project_name"
render = { type = "animated_postfx", inner = "text_ascii", effect = "coalesce", duration_ms = 1500, style = "figlet", font = "banner", color = "panel_title", align = "center" }

[[widget]]
id = "subtitle"
fetcher = "project"
render = { type = "text_plain", align = "center" }

[[widget]]
id = "repo_stats"
fetcher = "github_repo_stars"
render = { type = "grid_table", layout = "inline", align = "center" }

[[widget]]
id = "repo_prs"
fetcher = "github_repo_prs"
render = { type = "list_plain", bullet = "•", max_items = 5 }

[[widget]]
id = "repo_issues"
fetcher = "github_repo_issues"
render = { type = "list_plain", bullet = "•", max_items = 5 }

[[widget]]
id = "commits_heatmap"
fetcher = "git_commits_activity"
render = "grid_heatmap"

[[widget]]
id = "top_contributors"
fetcher = "git_contributors"
render = { type = "chart_bar", max_bars = 5 }

[[widget]]
id = "languages"
fetcher = "github_languages"
render = { type = "chart_bar", horizontal = true }

[[widget]]
id = "releases"
fetcher = "github_recent_releases"
render = { type = "list_timeline", bullet = "●", date_format = "relative", max_items = 3 }

[[widget]]
id = "gap"
fetcher = "basic_static"
format = ""
render = "text_plain"

[[row]]
height = { length = 1 }
bg = "subtle"

[[row]]
height = { length = 8 }
bg = "subtle"
  [[row.child]]
  widget = "hero"

[[row]]
height = { length = 3 }
bg = "subtle"
flex = "center"
  [[row.child]]
  widget = "subtitle"
  width = { length = 70 }

[[row]]
height = { length = 1 }
bg = "subtle"
flex = "center"
  [[row.child]]
  widget = "repo_stats"
  width = { length = 70 }

[[row]]
height = { length = 1 }
bg = "subtle"

[[row]]
height = { length = 1 }

[[row]]
height = { length = 6 }
flex = "center"
  [[row.child]]
  widget = "languages"
  width = { length = 34 }
  border = "top"
  title = "languages"
  [[row.child]]
  widget = "gap"
  width = { length = 2 }
  [[row.child]]
  widget = "releases"
  width = { length = 34 }
  border = "top"
  title = "released"

[[row]]
height = { length = 1 }

[[row]]
height = { length = 9 }
flex = "center"
  [[row.child]]
  widget = "repo_prs"
  width = { length = 34 }
  border = "top"
  title = "PRs"
  [[row.child]]
  widget = "gap"
  width = { length = 2 }
  [[row.child]]
  widget = "repo_issues"
  width = { length = 34 }
  border = "top"
  title = "issues"

[[row]]
height = { length = 1 }

[[row]]
height = { length = 10 }
flex = "center"
  [[row.child]]
  widget = "commits_heatmap"
  width = { length = 34 }
  border = "top"
  title = "commits"
  [[row.child]]
  widget = "gap"
  width = { length = 2 }
  [[row.child]]
  widget = "top_contributors"
  width = { length = 34 }
  border = "top"
  title = "top contributors"
```

## Applying a preset

Copy the TOML into the right file for the context:

- `home_*` → `$HOME/.splashboard/home.dashboard.toml`
- `project_*` → `./.splashboard/dashboard.toml` (ships with the repo), or
  `$HOME/.splashboard/project.dashboard.toml` (applies to every repo root
  that doesn't ship its own dashboard).

A `splashboard install --template <name>` helper is planned
([issue #58](https://github.com/unhappychoice/splashboard/issues/58))
that picks a preset from a gallery and writes it to the right location
for the chosen context — along with shell init. Until then, copy-paste is
the supported path.
