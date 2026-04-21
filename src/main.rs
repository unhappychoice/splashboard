use std::io::{self, BufRead, IsTerminal, Write, stdin, stdout};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::config::{Config, WidgetConfig};
use crate::fetcher::{Registry, Safety};
use crate::shell::Shell;
use crate::trust::{TrustStore, load_config_and_hash};

mod cache;
mod config;
mod daemon;
mod fetcher;
mod layout;
mod payload;
mod render;
mod runtime;
mod shell;
mod trust;

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
    /// Grant this project-local config permission to run Network and Exec widgets. Safe widgets
    /// always run regardless; this is the consent step for anything that talks to the outside
    /// world. Defaults to the nearest `.splashboard.toml` walking up from the current directory.
    Trust {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Remove trust for a project-local config. Network and Exec widgets in it will render the
    /// "🔒 requires trust" placeholder until re-trusted.
    Revoke {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Print the currently trusted local configs.
    ListTrusted,
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
        Some(Command::Trust { path }) => run_trust(path),
        Some(Command::Revoke { path }) => run_revoke(path),
        Some(Command::ListTrusted) => run_list_trusted(),
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
    let (config, ident) = load_full_config()?;
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::run(&config, ident_ref, wait).await
}

async fn render_for_cd(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let Some(path) = config::resolve_cwd_only_path() else {
        return Ok(());
    };
    let (config, hash) = load_config_and_hash(&path).map_err(io::Error::other)?;
    runtime::run(&config, Some((&path, &hash)), wait).await
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

/// Returns the resolved config plus an optional `(path, hash)` pair. Missing file / baked
/// default produces `None` — trust gating treats that as implicitly trusted. When a file is
/// present, reading and hashing happen in a single [`load_config_and_hash`] call so the trust
/// check can't be TOCTOU'd between parse and hash.
fn load_full_config() -> io::Result<(Config, Option<(PathBuf, String)>)> {
    match config::resolve_config_path() {
        Some(p) => {
            let (config, hash) = load_config_and_hash(&p).map_err(io::Error::other)?;
            Ok((config, Some((p, hash))))
        }
        None => Ok((Config::default_baked(), None)),
    }
}

fn run_trust(path: Option<PathBuf>) -> io::Result<()> {
    let Some(target) = resolve_trust_target(path) else {
        eprintln!(
            "no project-local config found (run from inside a directory with .splashboard.toml)"
        );
        return Ok(());
    };
    // Read the bytes once so the hash we show the user matches the hash we store — and so an
    // attacker can't swap the file between "here's what it asks for" and "ok I trust it".
    let (config, hash) = load_config_and_hash(&target).map_err(io::Error::other)?;
    let registry = Registry::with_builtins();
    print_trust_summary(&target, &hash, &config.widgets, &registry)?;
    if !prompt_yes_no("Trust this config?")? {
        println!("not trusted");
        return Ok(());
    }
    let mut store = TrustStore::load();
    store.trust(&target, hash)?;
    println!("trusted: {}", target.display());
    Ok(())
}

fn run_revoke(path: Option<PathBuf>) -> io::Result<()> {
    let Some(target) = resolve_trust_target(path) else {
        eprintln!("no project-local config found");
        return Ok(());
    };
    let mut store = TrustStore::load();
    let display = sanitize_for_display(&target.display().to_string());
    if store.revoke(&target)? {
        println!("revoked: {display}");
    } else {
        println!("not trusted: {display}");
    }
    Ok(())
}

fn run_list_trusted() -> io::Result<()> {
    let store = TrustStore::load();
    for entry in store.list() {
        println!(
            "{}  {}",
            entry.sha256,
            sanitize_for_display(&entry.path.display().to_string())
        );
    }
    Ok(())
}

fn resolve_trust_target(override_path: Option<PathBuf>) -> Option<PathBuf> {
    override_path.or_else(config::resolve_local_config_path)
}

fn print_trust_summary(
    path: &Path,
    hash: &str,
    widgets: &[WidgetConfig],
    registry: &Registry,
) -> io::Result<()> {
    // Paths and widget ids/fetchers flow into the terminal unmodified; sanitize control chars so
    // a malicious config can't spoof the prompt with ANSI escape sequences.
    println!(
        "Config: {}",
        sanitize_for_display(&path.display().to_string())
    );
    println!("sha256: {hash}");
    println!();

    let mut declared = 0usize;
    for w in widgets {
        let Some(fetcher) = registry.get(&w.fetcher) else {
            continue;
        };
        let label = match fetcher.safety() {
            Safety::Safe => continue,
            Safety::Network => "network",
            Safety::Exec => "exec",
        };
        if declared == 0 {
            println!("This config requests:");
        }
        println!(
            "  - {label:<7}: {} ({})",
            sanitize_for_display(&w.id),
            sanitize_for_display(&w.fetcher)
        );
        declared += 1;
    }
    if declared == 0 {
        println!("(no Network or Exec widgets — nothing to trust)");
    }
    println!();
    Ok(())
}

/// Replaces control characters (including ANSI escape initiators) with U+FFFD so a hostile
/// config can't draw over the trust prompt to make it look like something else.
fn sanitize_for_display(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect()
}

fn prompt_yes_no(question: &str) -> io::Result<bool> {
    print!("{question} [y/N] ");
    stdout().flush()?;
    let mut line = String::new();
    stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
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

    #[test]
    fn sanitize_replaces_control_chars() {
        let evil = "legit\x1b[2Kspoof";
        let safe = super::sanitize_for_display(evil);
        assert!(!safe.contains('\x1b'));
        assert!(safe.contains('\u{FFFD}'));
    }

    #[test]
    fn sanitize_preserves_normal_text() {
        let s = super::sanitize_for_display("hello/world-dashboard_01");
        assert_eq!(s, "hello/world-dashboard_01");
    }

    #[test]
    fn sanitize_replaces_newline_and_tab() {
        let s = super::sanitize_for_display("a\nb\tc");
        assert_eq!(s.matches('\u{FFFD}').count(), 2);
    }
}
