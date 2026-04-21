use std::io::{self, IsTerminal, stdin, stdout};
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::shell::Shell;

mod cache;
mod config;
mod daemon;
mod fetcher;
mod layout;
mod payload;
mod render;
mod runtime;
mod shell;

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

    /// Wait for fresh data before drawing (skips the cache-first fast path). Slower startup,
    /// guarantees the frame reflects current values. Equivalent to `general.wait_for_fresh`.
    #[arg(long)]
    wait: bool,

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
    /// Internal: run fetchers and update the cache. Spawned as a detached child by the main
    /// splashboard invocation; not intended to be run directly.
    #[command(hide = true)]
    FetchOnly {
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init { shell }) => {
            print!("{}", shell::init_snippet(shell));
            Ok(())
        }
        Some(Command::FetchOnly { config }) => daemon::run_fetch_only(config.as_deref()).await,
        None => {
            // Swallow render errors at the shell-facing boundary so a broken splash never breaks
            // the user's prompt. Internal paths (FetchOnly above) still propagate errors.
            let _ = if cli.on_cd {
                render_for_cd(cli.wait).await
            } else {
                render_splash(cli.wait).await
            };
            Ok(())
        }
    }
}

async fn render_splash(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let (config, path) = load_full_config()?;
    runtime::run(&config, path.as_deref(), wait).await
}

async fn render_for_cd(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let Some(path) = config::resolve_cwd_only_path() else {
        return Ok(());
    };
    let config = Config::load_or_default(&path).map_err(io::Error::other)?;
    runtime::run(&config, Some(&path), wait).await
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

fn load_full_config() -> io::Result<(Config, Option<PathBuf>)> {
    let path = config::resolve_config_path();
    let config = match &path {
        Some(p) => Config::load_or_default(p).map_err(io::Error::other)?,
        None => Config::default_baked(),
    };
    Ok((config, path))
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
