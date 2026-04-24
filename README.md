# splashboard

[![CI](https://github.com/unhappychoice/splashboard/actions/workflows/ci.yml/badge.svg)](https://github.com/unhappychoice/splashboard/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/unhappychoice/splashboard/branch/main/graph/badge.svg)](https://codecov.io/gh/unhappychoice/splashboard)

<p align="center">
  <img src="docs/screenshots/project_github.png" alt="splashboard project_github preset" width="820">
</p>

A customizable terminal splash rendered on shell startup and on `cd`.

> `splashboard` = `splash` + `dashboard`.

Instead of a blinking cursor, every new shell shows a dashboard of the things you actually care about — greetings, git status, CI health, PRs, a contributions heatmap, the moon phase. The killer feature: a repo that ships `./.splashboard/dashboard.toml` auto-reshapes the splash when you `cd` in, so different repos get different splashes for free.

## Install

```bash
# cargo
cargo install splashboard

# Homebrew (macOS / Linux)
brew install unhappychoice/tap/splashboard

# cargo-binstall (prebuilt binaries from GitHub Releases)
cargo binstall splashboard

# One-liner install script (Linux / macOS)
curl -fsSL https://raw.githubusercontent.com/unhappychoice/splashboard/main/install.sh | bash
```

Prebuilt binaries for Linux (x86_64 / aarch64), macOS (x86_64 / aarch64), and Windows (x86_64) are also attached to each [GitHub Release](https://github.com/unhappychoice/splashboard/releases).

After install, run:

```bash
splashboard install
```

`splashboard install` detects your shell, walks you through template / theme pickers, and wires your rc for you.

Prefer to own the rc edit yourself? Append one line that re-sources `splashboard init <shell>` on every shell start — upgrades to splashboard ship an updated init snippet automatically:

```bash
echo 'eval "$(splashboard init zsh)"'                              >> ~/.zshrc
echo 'eval "$(splashboard init bash)"'                             >> ~/.bashrc
echo 'splashboard init fish | source'                              >> ~/.config/fish/config.fish
echo 'Invoke-Expression (& splashboard init powershell | Out-String)' >> $PROFILE
```

## Docs

📖 **<https://unhappychoice.github.io/splashboard/>**

- [Getting started](https://unhappychoice.github.io/splashboard/guides/getting-started/) — install, wire your shell, render your first splash
- [Concepts](https://unhappychoice.github.io/splashboard/guides/concepts/) — the mental model (Widget = Fetcher + Renderer + Layout slot)
- [Configuration](https://unhappychoice.github.io/splashboard/guides/configuration/) — the full TOML schema
- [Presets](https://unhappychoice.github.io/splashboard/guides/presets/) & [Themes](https://unhappychoice.github.io/splashboard/guides/themes/) — curated dashboards and palettes
- [Trust model](https://unhappychoice.github.io/splashboard/guides/trust/) — how per-directory configs are sandboxed
- [Reference](https://unhappychoice.github.io/splashboard/reference/matrix/) — every fetcher and renderer with options and compatible shapes

## Status

Usable day-to-day. Widget catalog tracked as a living roadmap in [issue #41](https://github.com/unhappychoice/splashboard/issues/41) — new fetchers and renderers land as PRs that tick the checkboxes.

## License

ISC

## Related

- [gitlogue](https://github.com/unhappychoice/gitlogue) — cinematic git history replay
- [gittype](https://github.com/unhappychoice/gittype) — CLI typing game from your source code
- [mdts](https://github.com/unhappychoice/mdts) — local Markdown tree server
