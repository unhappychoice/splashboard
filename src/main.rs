use std::io::{self, BufRead, IsTerminal, Write, stdin, stdout};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use splashboard::catalog;
use splashboard::config::{
    self, Config, DashboardConfig, DashboardSource, SettingsConfig, WidgetConfig,
};
use splashboard::daemon::{self, DashboardKind};
use splashboard::fetcher::{Registry, Safety};
use splashboard::paths;
use splashboard::render::Registry as RenderRegistry;
use splashboard::runtime;
use splashboard::shell::{self, Shell};
use splashboard::trust::{TrustStore, load_dashboard_and_hash};

const OPT_OUT_ENV_VARS: &[&str] = &["CI", "SPLASHBOARD_SILENT", "NO_SPLASHBOARD"];
const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 16;

#[derive(Parser)]
#[command(version, about = "A customizable terminal splash screen")]
struct Cli {
    /// Render only if the current directory directly resolves to a dashboard (per-dir file or
    /// git repo root); otherwise exit silently. Intended for cd-hook invocations so the splash
    /// shows exactly once per project entry instead of on every subdirectory navigation.
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
    /// Grant this project-local dashboard permission to run Network widgets. Safe widgets
    /// always run regardless; this is the consent step for anything that talks to the outside
    /// world. Defaults to the nearest `.splashboard.toml` walking up from the current directory.
    Trust {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Remove trust for a project-local dashboard. Network widgets in it will render the
    /// "🔒 requires trust" placeholder until re-trusted.
    Revoke {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Print the currently trusted local dashboards.
    ListTrusted,
    /// Browse the built-in fetcher and renderer catalog — the same info the docs site exposes,
    /// rendered for the terminal. Run without a target for an overview; use
    /// `catalog fetcher [NAME]` or `catalog renderer [NAME]` to narrow.
    Catalog {
        #[command(subcommand)]
        target: Option<CatalogTarget>,
    },
    /// Internal: run fetchers and update the cache. Spawned as a detached child by the main
    /// splashboard invocation; not intended to be run directly.
    #[command(hide = true)]
    FetchOnly {
        #[arg(long, value_enum)]
        kind: DashboardKind,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CatalogTarget {
    /// List all fetchers, or show details for one when NAME is given.
    #[command(alias = "fetchers")]
    Fetcher { name: Option<String> },
    /// List all renderers, or show details for one when NAME is given.
    #[command(alias = "renderers")]
    Renderer { name: Option<String> },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init { shell }) => {
            print!("{}", shell::init_snippet(shell));
            Ok(())
        }
        Some(Command::FetchOnly { kind, path }) => {
            daemon::run_fetch_only(kind, path.as_deref()).await
        }
        Some(Command::Trust { path }) => run_trust(path),
        Some(Command::Revoke { path }) => run_revoke(path),
        Some(Command::ListTrusted) => run_list_trusted(),
        Some(Command::Catalog { target }) => run_catalog(target),
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
    let source = config::resolve_dashboard_source();
    let (config, ident) = load_full_config(&source)?;
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::run(&config, &source, ident_ref, wait).await
}

async fn render_for_cd(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let Some(source) = config::resolve_on_cd_dashboard_source() else {
        return Ok(());
    };
    let (config, ident) = load_full_config(&source)?;
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::run(&config, &source, ident_ref, wait).await
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

/// Loads settings + the resolved dashboard and composes them into a `Config`. The optional
/// `(path, hash)` identifies a local dashboard for trust gating; HOME-backed sources return
/// `None` so they're treated as implicitly trusted.
fn load_full_config(source: &DashboardSource) -> io::Result<(Config, Option<(PathBuf, String)>)> {
    let settings = load_settings()?;
    let (dashboard, ident) = match source {
        DashboardSource::Local(p) => {
            let (d, h) = load_dashboard_and_hash(p).map_err(io::Error::other)?;
            (d, Some((p.clone(), h)))
        }
        DashboardSource::Home => (load_home_dashboard_or_baked()?, None),
        DashboardSource::Project => (load_project_dashboard_or_baked()?, None),
    };
    Ok((Config::from_parts(settings, dashboard), ident))
}

fn load_settings() -> io::Result<SettingsConfig> {
    match paths::settings_path() {
        Some(p) => SettingsConfig::load_or_default(&p).map_err(io::Error::other),
        None => Ok(SettingsConfig::default_baked()),
    }
}

fn load_home_dashboard_or_baked() -> io::Result<DashboardConfig> {
    load_dashboard_file_or(paths::home_dashboard_path(), DashboardConfig::default_home)
}

fn load_project_dashboard_or_baked() -> io::Result<DashboardConfig> {
    load_dashboard_file_or(
        paths::project_dashboard_path(),
        DashboardConfig::default_project,
    )
}

fn load_dashboard_file_or(
    path: Option<PathBuf>,
    baked: impl FnOnce() -> DashboardConfig,
) -> io::Result<DashboardConfig> {
    let Some(path) = path else {
        return Ok(baked());
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => DashboardConfig::parse(&s)
            .map_err(|e| io::Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(baked()),
        Err(e) => Err(e),
    }
}

fn run_trust(path: Option<PathBuf>) -> io::Result<()> {
    let Some(target) = resolve_trust_target(path) else {
        eprintln!(
            "no project-local dashboard found (run from inside a directory with .splashboard.toml)"
        );
        return Ok(());
    };
    // Read the bytes once so the hash we show the user matches the hash we store — and so an
    // attacker can't swap the file between "here's what it asks for" and "ok I trust it".
    let (dashboard, hash) = load_dashboard_and_hash(&target).map_err(io::Error::other)?;
    let registry = Registry::with_builtins();
    print_trust_summary(&target, &hash, &dashboard.widgets, &registry)?;
    if !prompt_yes_no("Trust this dashboard?")? {
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
        eprintln!("no project-local dashboard found");
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

fn run_catalog(target: Option<CatalogTarget>) -> io::Result<()> {
    let fetchers = Registry::with_builtins();
    let renderers = RenderRegistry::with_builtins();
    let output = match target {
        None => Ok(catalog::overview(&fetchers, &renderers)),
        Some(CatalogTarget::Fetcher { name: None }) => Ok(catalog::fetcher_list(&fetchers)),
        Some(CatalogTarget::Fetcher { name: Some(n) }) => {
            catalog::fetcher_detail(&n, &fetchers, &renderers)
        }
        Some(CatalogTarget::Renderer { name: None }) => Ok(catalog::renderer_list(&renderers)),
        Some(CatalogTarget::Renderer { name: Some(n) }) => {
            catalog::renderer_detail(&n, &renderers, &fetchers)
        }
    };
    match output {
        Ok(s) => {
            print!("{s}");
            Ok(())
        }
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    }
}

fn resolve_trust_target(override_path: Option<PathBuf>) -> Option<PathBuf> {
    override_path.or_else(config::resolve_local_dashboard_path)
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
        "Dashboard: {}",
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
            println!("This dashboard requests:");
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
