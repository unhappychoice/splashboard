use std::io::{self, IsTerminal, stdin, stdout};

use clap::{Parser, Subcommand};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};

use crate::config::Config;
use crate::shell::Shell;

mod config;
mod layout;
mod payload;
mod render;
mod shell;
mod stubs;

const OPT_OUT_ENV_VARS: &[&str] = &["CI", "SPLASHBOARD_SILENT", "NO_SPLASHBOARD"];
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 16;

#[derive(Parser)]
#[command(version, about = "A customizable terminal splash screen")]
struct Cli {
    /// Render only if the current directory directly holds a config file; otherwise exit
    /// silently. Intended for cd-hook invocations so the splash shows exactly once per project
    /// entry instead of on every subdirectory navigation.
    #[arg(long)]
    on_cd: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Emit a shell init snippet; source it from your rc file to render on new shells and on cd.
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init { shell }) => {
            print!("{}", shell::init_snippet(shell));
            Ok(())
        }
        None => {
            let _ = if cli.on_cd {
                render_for_cd()
            } else {
                render_splash()
            };
            Ok(())
        }
    }
}

fn render_splash() -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    draw(&load_full_config())
}

fn render_for_cd() -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let Some(path) = config::resolve_cwd_only_path() else {
        return Ok(());
    };
    let Ok(config) = Config::load_or_default(&path) else {
        return Ok(());
    };
    draw(&config)
}

fn should_render() -> bool {
    stdout().is_terminal()
        && stdin().is_terminal()
        && allow_render(|k| std::env::var(k).ok())
        && meets_minimum_size()
}

fn allow_render(env: impl Fn(&str) -> Option<String>) -> bool {
    if OPT_OUT_ENV_VARS.iter().any(|k| env(k).is_some()) {
        return false;
    }
    !matches!(env("TERM").as_deref(), Some("dumb"))
}

fn meets_minimum_size() -> bool {
    ratatui::crossterm::terminal::size()
        .map(|(w, h)| is_large_enough(w, h))
        .unwrap_or(false)
}

fn is_large_enough(width: u16, height: u16) -> bool {
    width >= MIN_WIDTH && height >= MIN_HEIGHT
}

fn draw(config: &Config) -> io::Result<()> {
    let root = config.to_layout();
    let widgets = stubs::widgets_for(config.widgets.iter().map(|w| &w.id));
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(16),
        },
    )?;
    terminal.draw(|frame| layout::draw(frame, frame.area(), &root, &widgets))?;
    println!();
    Ok(())
}

fn load_full_config() -> Config {
    config::resolve_config_path()
        .and_then(|p| Config::load_or_default(&p).ok())
        .unwrap_or_else(Config::default_baked)
}

#[cfg(test)]
mod tests {
    use super::allow_render;

    fn env_with(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn allows_render_in_plain_env() {
        assert!(allow_render(env_with(&[])));
    }

    #[test]
    fn ci_env_blocks_render() {
        assert!(!allow_render(env_with(&[("CI", "true")])));
    }

    #[test]
    fn splashboard_silent_blocks_render() {
        assert!(!allow_render(env_with(&[("SPLASHBOARD_SILENT", "1")])));
    }

    #[test]
    fn no_splashboard_blocks_render() {
        assert!(!allow_render(env_with(&[("NO_SPLASHBOARD", "1")])));
    }

    #[test]
    fn dumb_terminal_blocks_render() {
        assert!(!allow_render(env_with(&[("TERM", "dumb")])));
    }

    #[test]
    fn normal_term_allows_render() {
        assert!(allow_render(env_with(&[("TERM", "xterm-256color")])));
    }

    #[test]
    fn large_enough_size_passes() {
        assert!(super::is_large_enough(80, 24));
        assert!(super::is_large_enough(super::MIN_WIDTH, super::MIN_HEIGHT));
    }

    #[test]
    fn below_min_width_fails() {
        assert!(!super::is_large_enough(39, 40));
    }

    #[test]
    fn below_min_height_fails() {
        assert!(!super::is_large_enough(80, 15));
    }
}
