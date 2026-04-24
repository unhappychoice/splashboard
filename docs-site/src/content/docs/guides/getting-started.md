---
title: Getting started
description: Install splashboard, wire it into your shell, and render your first splash.
---

splashboard renders a customizable TUI splash on every new shell and on `cd`.
This page walks through installing the binary, running the one-shot installer,
and verifying the first splash. It assumes nothing more than a working terminal.

## Install the binary

splashboard is distributed on crates.io. A stable Rust toolchain is the only
prerequisite.

```bash
cargo install splashboard
```

## Run `splashboard install`

```bash
splashboard install
```

The installer auto-detects your shell and walks you through four gallery
pickers — each with a live preview rendered from the shipped templates and
your chosen theme — followed by a final confirmation page:

1. **Home dashboard** — one of `home_splash` / `home_daily` / `home_github` /
   `home_minimal`. Shown on new shells and when `cd`-ing anywhere outside a
   git repo root.
2. **Project dashboard** — one of `project_splash` / `project_github` /
   `project_minimal`. Shown when `cd`-ing into the top of any git repo that
   doesn't ship its own `./.splashboard/`.
3. **Theme** — `default` (the Splash signature ocean palette), or one of
   `tokyo_night`, `catppuccin_mocha`, `dracula`, `nord`, `gruvbox_dark`. The
   preview re-renders your home pick under the highlighted theme so you see
   the combined look before committing.
4. **Defaults** — two toggles: whether the splash paints its own background
   (off = let the terminal's own background show through, useful on light
   or transparent terminals), and whether to block the first render until
   every widget has fetched at least once (default: off — cached-first
   paint, refresh in the background).
5. **Review and confirm** — the resolved plan (shell, templates, theme,
   toggles, and the exact files that will be touched). Enter / `y` writes
   them; Esc / `q` / `n` cancels without touching a single file.

Once you confirm, the installer writes `home.dashboard.toml` +
`project.dashboard.toml` + a starter `settings.toml` to `$HOME/.splashboard/`,
then appends a marker-delimited block to your shell rc that sources
`splashboard init <shell>`.

Re-running the installer is safe:

- Existing files whose content still matches the new template / theme are
  left as `unchanged` — no churn.
- Existing files that differ from what the installer is about to write are
  renamed to a `.bak` sidecar (`settings.toml` → `settings.toml.bak`) before
  the new content lands, so your previous state is always one `mv` away.
- The shell rc keeps a marker-delimited block that gets replaced in place
  rather than duplicated, and is created if missing.

### Scripted / non-TTY install

For dotfiles bootstrap (no pickers, deterministic output):

```bash
splashboard install \
  --shell zsh \
  --home-template home_splash \
  --project-template project_github \
  --theme tokyo_night \
  --no-bg \
  --wait
```

Flag reference:

| flag | writes to `settings.toml` | default |
|---|---|---|
| `--theme <preset>` | `[theme] preset = "..."` | `default` (no `preset` line) |
| `--no-bg` | `[theme] bg = "reset"` + `bg_subtle = "reset"` | splash paints its own bg |
| `--wait` | `[general] wait_for_fresh = true` | off (cached-first paint) |

Available templates:

- **home**: `home_splash`, `home_daily`, `home_github`, `home_minimal`
- **project**: `project_splash`, `project_github`, `project_minimal`

See [Presets](/splashboard/guides/presets/) for a rendered preview of each,
and [Themes](/splashboard/guides/themes/) for the palette details.

### Prefer to own the rc edit yourself?

If you manage your rc from a dotfile repo and want the splashboard line
source-tracked alongside everything else, add one line that re-sources
`splashboard init <shell>` on every shell start. This is the same line
`splashboard install` writes, and it means upgrades to splashboard ship an
updated snippet automatically — no re-run of `install` required.

```bash
# ~/.zshrc
eval "$(splashboard init zsh)"

# ~/.bashrc
eval "$(splashboard init bash)"

# ~/.config/fish/config.fish
splashboard init fish | source

# $PROFILE (PowerShell)
Invoke-Expression (& splashboard init powershell | Out-String)
```

Or, if you'd rather source-track the raw snippet itself in your dotfile
repo, dump the output into your rc once (re-run to pick up upgrades):

```bash
splashboard init zsh        >> ~/.zshrc
splashboard init bash       >> ~/.bashrc
splashboard init fish       >> ~/.config/fish/config.fish
splashboard init powershell >> $PROFILE
```

## The first splash

Open a new shell — you should see the preset you picked. The shell rc is now
wired so every new interactive shell renders a splash, and `cd` into a git
repo root repaints to your `project_*` preset.

Customize freely — dashboards are plain TOML:

```toml
# $HOME/.splashboard/home.dashboard.toml

[[widget]]
id = "clock"
fetcher = "clock"
render = { type = "text_ascii", pixel_size = "quadrant", align = "center" }

[[row]]
height = { length = 4 }
  [[row.child]]
  widget = "clock"
```

Run `splashboard` directly to re-render without waiting for a new shell.

From here:

- [Concepts](/splashboard/guides/concepts/) — the mental model (Widget =
  Fetcher + Renderer + Layout slot). Read this next if the TOML above
  looks a bit opaque; it unlocks the rest of the docs.
- [Configuration](/splashboard/guides/configuration/) — the full TOML
  schema (settings, dashboards, widgets, rows, render options).
- [Presets](/splashboard/guides/presets/) — curated dashboards you can
  adopt verbatim.
- [Themes](/splashboard/guides/themes/) — six built-in palettes plus
  per-token overrides.
- The [reference](/splashboard/reference/matrix/) — the complete list of
  fetchers and renderers with their options and compatible shapes.

## Troubleshooting

**Nothing renders.** splashboard deliberately opts out of rendering in
contexts where a splash would be noise rather than signal. The splash skips
rendering when any of these is true:

- `stdout` is not a terminal (piped, redirected, non-interactive).
- `TERM=dumb`.
- Any of `CI`, `SPLASHBOARD_SILENT`, or `NO_SPLASHBOARD` is set.
- The terminal is smaller than 40×16.

Unset the variable or widen the terminal and try again.

**A widget renders `🔒 requires trust`.** The widget is classified as
`Network` and the local dashboard it came from has not been trusted yet. See
the [trust model guide](/splashboard/guides/trust/) for the consent flow —
`splashboard trust` is usually what you want.

**A GitHub widget renders an error.** `github_*` fetchers need a token with
read access. Export `GH_TOKEN` (or `GITHUB_TOKEN`) before starting the shell.
See the [cookbook](/splashboard/guides/cookbook/#github-credentials) for the
recommended place to put it.

**I want to see fresh data, not cached.** Run `splashboard --wait` once. The
cached-first model normally paints instantly from the last refresh; `--wait`
blocks until every widget has fetched fresh.
