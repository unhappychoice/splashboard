# splashboard

A customizable terminal splash screen with plugin-based data sources.

> `splashboard` = `splash` + `dashboard` — a splash screen for your shell, rendered as a dashboard.

## What is this

Every time you open a terminal, you see... a blinking cursor.
What if instead, you saw a dashboard of the things you actually care about?

`splashboard` renders a customizable TUI splash screen on shell startup — greetings, git status, system info, GitHub notifications, weather, RSS, calendar, whatever you wire up. One line in your `.bashrc` / `.zshrc` / `config.fish` and you're done.

## Goals

- **Zero friction install** — one-line shell rc snippet
- **Blazing fast startup** — cached-first render, async refresh in background
- **Plugin ecosystem** — subprocess-based plugins, write them in any language
- **Beautiful by default** — built on Ratatui, pleasing out of the box
- **Customizable** — TOML config, themeable, layout control

## Design sketch

- **Language**: Rust
- **TUI**: [Ratatui](https://ratatui.rs/) + crossterm
- **Config**: TOML at `~/.config/splashboard/config.toml`
- **Cache**: `~/.cache/splashboard/` (per-widget, TTL-based)
- **Plugins**: executables in `~/.config/splashboard/plugins/`, stdin=JSON request, stdout=JSON response

### Lazy / cached rendering

Startup MUST NOT block the shell. The pattern:

1. On launch, read cached data and render immediately
2. Spawn async tasks to refresh data in background
3. Persist refreshed data for the next invocation

This way even HTTP-backed widgets (GitHub, weather, RSS) feel instant.

## Built-in widgets (planned)

### Sync (fast, local)
- Clock / date / greeting
- Git status (branch, dirty, ahead/behind)
- System info (OS, uptime, CPU/mem)
- TODO list (from local Markdown)

### Async (HTTP, lazy-cached)
- GitHub notifications / PRs awaiting review
- Weather
- RSS feed
- Calendar (iCal / Google)

### Plugins (user-defined)
- Anything you can express as "run an executable, get JSON back"
- Bash, Python, Go, Node, Ruby, whatever

## Plugin protocol (draft)

Plugins live at `~/.config/splashboard/plugins/<name>` and are executables.

**Input** (stdin, JSON):
```json
{
  "config": { ... per-plugin user config ... },
  "context": { "tty_width": 120, "tty_height": 40 }
}
```

**Output** (stdout, JSON):
```json
{
  "title": "My Plugin",
  "lines": ["line 1", "line 2"],
  "color": "cyan",
  "icon": "📦"
}
```

Language-agnostic. Write a plugin in Bash in 5 lines.

## Installation (planned)

```bash
# Install
cargo install splashboard
# or
brew install unhappychoice/tap/splashboard

# Wire into your shell
splashboard init bash >> ~/.bashrc
# splashboard init zsh >> ~/.zshrc
# splashboard init fish >> ~/.config/fish/config.fish
```

## Status

Early design phase. See [Issues](https://github.com/unhappychoice/splashboard/issues) for the roadmap.

## License

TBD (likely MIT or Apache-2.0)

## Related

- [gitlogue](https://github.com/unhappychoice/gitlogue) — cinematic git history replay
- [gittype](https://github.com/unhappychoice/gittype) — CLI typing game from your source code
- [mdts](https://github.com/unhappychoice/mdts) — local Markdown tree server
