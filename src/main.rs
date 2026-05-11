use std::future::Future;
use std::io::{self, BufRead, IsTerminal, Write, stdin, stdout};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use clap::{Parser, Subcommand};

use splashboard::cache::{self, CacheEntry};
use splashboard::catalog;
use splashboard::config::{
    self, Config, DashboardConfig, DashboardSource, SettingsConfig, WidgetConfig,
};
use splashboard::daemon::{self, DashboardKind};
use splashboard::fetcher::{FetchContext, Registry, Safety};
use splashboard::install::{self, InstallOptions};
use splashboard::logging;
use splashboard::paths;
use splashboard::render::Registry as RenderRegistry;
use splashboard::runtime;
use splashboard::secrets::SecretsConfig;
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
    /// One-shot onboarding: pick a home/project template pair, write them to
    /// `$HOME/.splashboard/`, and wire your shell rc so the splash renders on every new shell.
    /// Runs an interactive gallery picker in a TTY; non-TTY callers must pass
    /// `--home-template` and `--project-template`. Existing files are always backed up to
    /// `.bak` sidecars before being overwritten, so re-running is safe.
    Install {
        /// Shell to wire. Omit to auto-detect from `$SHELL`.
        #[arg(long, value_enum)]
        shell: Option<Shell>,
        /// Home template name (e.g. `home_splash`). Required in non-TTY mode.
        #[arg(long)]
        home_template: Option<String>,
        /// Project template name (e.g. `project_github`). Required in non-TTY mode.
        #[arg(long)]
        project_template: Option<String>,
        /// Theme preset for `settings.toml`. One of `default`, `catppuccin_mocha`,
        /// `dracula`, `gruvbox_dark`, `nord`, `tokyo_night`.
        #[arg(long)]
        theme: Option<String>,
        /// Inherit the terminal's own background instead of painting the Splash palette.
        /// Writes `bg = "reset"` + `bg_subtle = "reset"` into `settings.toml`.
        #[arg(long)]
        no_bg: bool,
        /// Block the first render until every widget has fetched at least once. Writes
        /// `wait_for_fresh = true` into `settings.toml`.
        #[arg(long)]
        wait: bool,
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
    /// Print embedded license text. Defaults to the third-party dependency bundle
    /// (`THIRDPARTY-LICENSES.md`); pass `--own` for splashboard's own ISC LICENSE.
    License {
        /// Print splashboard's own ISC LICENSE instead of the third-party bundle.
        #[arg(long)]
        own: bool,
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
    /// Inspect and manage the disk cache (`$HOME/.splashboard/cache/`). Useful when a widget
    /// shows stale data, you need to force-refresh a single entry, or you want to know how much
    /// disk the cache is using.
    Cache {
        #[command(subcommand)]
        subcommand: CacheSubcommand,
    },
}

#[derive(Subcommand)]
enum CacheSubcommand {
    /// Print the resolved cache directory.
    Path,
    /// List every cache entry: key, age, TTL, fresh/stale, payload size, kind. Sorted by age
    /// descending. Pass `--json` for one JSON object per line.
    List {
        /// Emit one JSON object per entry instead of the human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Remove cache entries. Pass a widget id to remove only that widget's entry (the widget must
    /// still be configured); otherwise removes every entry plus any leftover `.lock` files.
    Clear {
        /// Widget id to clear. If omitted, clears every entry.
        #[arg(value_name = "WIDGET_ID")]
        widget_id: Option<String>,
        /// Skip the confirmation prompt for `clear` (no widget id).
        #[arg(long)]
        yes: bool,
        /// Emit a JSON summary instead of the human-readable lines.
        #[arg(long)]
        json: bool,
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

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init { shell }) => {
            print!("{}", shell::init_snippet(shell));
            Ok(())
        }
        Some(Command::Install {
            shell,
            home_template,
            project_template,
            theme,
            no_bg,
            wait,
        }) => install::run(InstallOptions {
            shell,
            home_template,
            project_template,
            theme,
            // Flag absence = "leave it to the picker / default"; flag presence = force the
            // opposite of the default. `--no-bg` disables bg, `--wait` enables wait.
            bg: no_bg.then_some(false),
            wait: wait.then_some(true),
            ..Default::default()
        }),
        Some(Command::FetchOnly { kind, path }) => run_async({
            logging::init();
            apply_secrets();
            daemon::run_fetch_only(kind, path.as_deref())
        }),
        Some(Command::Trust { path }) => run_trust(path),
        Some(Command::Revoke { path }) => run_revoke(path),
        Some(Command::ListTrusted) => run_list_trusted(),
        Some(Command::Catalog { target }) => run_catalog(target),
        Some(Command::License { own }) => run_license(own),
        Some(Command::Cache { subcommand }) => run_cache(subcommand),
        None => {
            if !should_render() {
                return Ok(());
            }
            logging::init();
            apply_secrets();
            // Swallow render errors at the shell-facing boundary so a broken splash never breaks
            // the user's prompt. Internal paths (FetchOnly above) still propagate errors.
            let _ = if cli.on_cd {
                run_async(render_for_cd(cli.wait))
            } else {
                run_async(render_splash(cli.wait))
            };
            Ok(())
        }
    }
}

fn run_async<F>(future: F) -> io::Result<()>
where
    F: Future<Output = io::Result<()>>,
{
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(future)
}

async fn render_splash(wait: bool) -> io::Result<()> {
    if !should_render() {
        return Ok(());
    }
    let source = config::resolve_dashboard_source();
    let (config, ident) = load_full_config(&source)?;
    if matches!(source, DashboardSource::Home) && !config.general.auto_home {
        return Ok(());
    }
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
    if !config.general.auto_on_cd {
        return Ok(());
    }
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

/// Loads `$HOME/.splashboard/secrets.toml` and exports each entry as a process env var, but
/// only when the env doesn't already define it. Runs once at startup so every fetcher
/// (`GH_TOKEN`, `TODOIST_TOKEN`, etc.) sees the same view regardless of who reads first.
/// Failures (parse error, unreadable file) log and continue — a stale token shouldn't break
/// the splash, and the user's shell env is still authoritative.
fn apply_secrets() {
    let Some(path) = paths::secrets_path() else {
        return;
    };
    match SecretsConfig::load_or_default(&path) {
        Ok(secrets) => {
            secrets.apply_to_env(
                |k| std::env::var(k).ok(),
                // SAFETY: called once from `main` before any user code reads env. The Tokio
                // worker threads exist but are idle — none of our fetchers / runtime tasks
                // have been spawned yet, so no thread is racing on `getenv`/`setenv`.
                |k, v| unsafe { std::env::set_var(k, v) },
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to load secrets.toml; continuing without it");
        }
    }
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

fn run_cache(subcommand: CacheSubcommand) -> io::Result<()> {
    let Some(dir) = paths::cache_dir() else {
        eprintln!("no cache dir resolved (is $HOME set?)");
        std::process::exit(2);
    };
    match subcommand {
        CacheSubcommand::Path => {
            println!("{}", dir.display());
            Ok(())
        }
        CacheSubcommand::List { json } => run_cache_list(&dir, json),
        CacheSubcommand::Clear {
            widget_id,
            yes,
            json,
        } => run_cache_clear(&dir, widget_id, yes, json),
    }
}

/// One row of the `cache list` output. Also serialized as a JSON object when `--json` is used.
#[derive(serde::Serialize)]
struct CacheListRow {
    /// Cache key — the filename stem, e.g. `clock-3f2a1c8b`.
    key: String,
    /// `entry` for `.json` payloads, `lock` for orphan `.lock` files.
    kind: &'static str,
    /// Seconds since the file was last written; for lock files this is the lock age.
    age_seconds: u64,
    /// TTL declared by the entry, in seconds. Zero for lock files.
    ttl_seconds: u64,
    /// `fresh` when `age_seconds < ttl_seconds`, otherwise `stale`. Lock files report `n/a`.
    freshness: &'static str,
    /// Size of the on-disk file, in bytes.
    size_bytes: u64,
    /// Outcome of the fetch that produced this entry — `ok` / `err` / `timeout`. `n/a` for locks.
    outcome: &'static str,
}

fn run_cache_list(dir: &Path, json: bool) -> io::Result<()> {
    let mut rows = collect_cache_rows(dir)?;
    // Oldest first by default — surfaces stale and orphan files at the top where they're easier
    // to spot when scrolling a long list.
    rows.sort_by(|a, b| b.age_seconds.cmp(&a.age_seconds));

    if json {
        for row in &rows {
            let line = serde_json::to_string(row).map_err(io::Error::other)?;
            println!("{line}");
        }
        return Ok(());
    }

    if rows.is_empty() {
        println!("(empty)");
        return Ok(());
    }

    println!(
        "{:<40}  {:<6}  {:>10}  {:>10}  {:<8}  {:>10}  {}",
        "KEY", "KIND", "AGE(s)", "TTL(s)", "STATE", "SIZE(B)", "OUTCOME"
    );
    for row in &rows {
        println!(
            "{:<40}  {:<6}  {:>10}  {:>10}  {:<8}  {:>10}  {}",
            truncate(&row.key, 40),
            row.kind,
            row.age_seconds,
            row.ttl_seconds,
            row.freshness,
            row.size_bytes,
            row.outcome,
        );
    }
    Ok(())
}

fn collect_cache_rows(dir: &Path) -> io::Result<Vec<CacheListRow>> {
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // Treat a missing cache dir the same as an empty one — operator hasn't run splashboard
        // yet, no cached data, nothing to report.
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(rows),
        Err(e) => return Err(e),
    };
    let now = SystemTime::now();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if !metadata.is_file() {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let age_seconds = metadata
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let size_bytes = metadata.len();
        match ext {
            "json" => {
                let entry = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<CacheEntry>(&s).ok());
                let (ttl, freshness, outcome) = entry
                    .as_ref()
                    .map(|e| {
                        let ttl = e.ttl_seconds;
                        let freshness = if e.is_fresh() { "fresh" } else { "stale" };
                        let outcome = match e.kind {
                            cache::CacheEntryKind::Ok => "ok",
                            cache::CacheEntryKind::Err => "err",
                            cache::CacheEntryKind::Timeout => "timeout",
                        };
                        (ttl, freshness, outcome)
                    })
                    .unwrap_or((0, "unreadable", "unreadable"));
                rows.push(CacheListRow {
                    key: stem.to_string(),
                    kind: "entry",
                    age_seconds,
                    ttl_seconds: ttl,
                    freshness,
                    size_bytes,
                    outcome,
                });
            }
            "lock" => {
                rows.push(CacheListRow {
                    key: stem.to_string(),
                    kind: "lock",
                    age_seconds,
                    ttl_seconds: 0,
                    freshness: "n/a",
                    size_bytes,
                    outcome: "n/a",
                });
            }
            // Anything else (`.tmp` leftovers, future formats) is skipped silently to keep the
            // output focused on the things users can act on.
            _ => continue,
        }
    }
    Ok(rows)
}

fn run_cache_clear(dir: &Path, widget_id: Option<String>, yes: bool, json: bool) -> io::Result<()> {
    match widget_id {
        Some(id) => clear_one(dir, &id, json),
        None => clear_all(dir, yes, json),
    }
}

fn clear_all(dir: &Path, yes: bool, json: bool) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e.collect::<Result<Vec<_>, _>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let targets: Vec<PathBuf> = entries
        .into_iter()
        .filter(|e| e.metadata().map(|m| m.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("json") | Some("lock")
            )
        })
        .collect();

    if targets.is_empty() {
        if json {
            println!(r#"{{"removed":0}}"#);
        } else {
            println!("(empty)");
        }
        return Ok(());
    }

    if !yes
        && !prompt_yes_no(&format!(
            "Remove all {} cache file(s) from {}?",
            targets.len(),
            dir.display()
        ))?
    {
        println!("cancelled");
        return Ok(());
    }

    let mut removed = 0usize;
    let mut errors = Vec::new();
    for path in &targets {
        match std::fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }

    if json {
        let payload = serde_json::json!({
            "removed": removed,
            "errors": errors,
        });
        println!("{payload}");
    } else {
        println!("removed {removed} file(s)");
        for err in &errors {
            eprintln!("error: {err}");
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{} file(s) could not be removed",
            errors.len()
        )))
    }
}

fn clear_one(dir: &Path, widget_id: &str, json: bool) -> io::Result<()> {
    // Use option 1 from the issue: compute the key from the current config. Falls back to the
    // home dashboard so the command works outside any project-local context (common case for
    // ad-hoc debugging on $HOME/.splashboard/cache/).
    let source = config::resolve_dashboard_source();
    let (config, _ident) = load_full_config(&source)?;
    let widget = config
        .widgets
        .iter()
        .find(|w| w.id == widget_id)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no widget '{widget_id}' in current config ({:?}). \
Cache key can only be computed for widgets that are still configured; \
to remove an orphaned entry run `splashboard cache list` and delete \
the file directly.",
                    source
                ),
            )
        })?;
    let registry = Registry::with_builtins();
    let fetcher = registry.get_cached(&widget.fetcher).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "widget '{}' uses fetcher '{}' which is not in the registry \
(realtime widgets don't have disk-cached entries)",
                widget.id, widget.fetcher,
            ),
        )
    })?;
    let key = fetcher.cache_key(&FetchContext {
        widget_id: widget.id.clone(),
        format: widget.format.clone(),
        // Cache key derivation does not actually look at timeout; pass zero to be explicit.
        timeout: std::time::Duration::from_secs(0),
        file_format: widget.file_format.clone(),
        // Shape is part of the key but we don't know which shape will be used at fetch time
        // without doing a layout pass. The default-shape path matches what `runtime::fetch_all`
        // uses when no widget-specific shape override is in play.
        shape: Some(fetcher.default_shape()),
        options: widget.options.clone(),
        // Locale and timezone do NOT participate in `default_cache_key` (see
        // `fetcher/mod.rs::default_cache_key`); a None pair here is correct and intentional,
        // and any fetcher that overrides `cache_key` to consume these would derive a key that
        // won't match a runtime-produced entry anyway.
        timezone: None,
        locale: None,
    });

    // Same sanitization as cache.rs; replicated here to avoid widening the public API for a
    // single character set that's stable across the codebase.
    let sanitized = sanitize_cache_key(&key);
    let entry_path = dir.join(format!("{sanitized}.json"));
    let lock_path = dir.join(format!("{sanitized}.lock"));
    let mut removed = Vec::new();
    let mut missing = Vec::new();
    for path in [&entry_path, &lock_path] {
        match std::fs::remove_file(path) {
            Ok(()) => removed.push(path.display().to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                missing.push(path.display().to_string());
            }
            Err(e) => {
                return Err(io::Error::other(format!(
                    "removing {}: {e}",
                    path.display()
                )));
            }
        }
    }

    if json {
        let payload = serde_json::json!({
            "widget_id": widget_id,
            "key": key,
            "removed": removed,
            "missing": missing,
        });
        println!("{payload}");
    } else if removed.is_empty() {
        println!("widget '{widget_id}' had no cache entry (key {key})");
    } else {
        println!(
            "removed {} file(s) for widget '{widget_id}' (key {key})",
            removed.len()
        );
        for path in &removed {
            println!("  {path}");
        }
    }
    Ok(())
}

/// Replicates `cache::sanitize` (which is module-private) so we can compute the on-disk
/// filename for a given cache key. Kept in sync with `cache.rs`; covered by the
/// `clear_one_matches_cache_filename_layout` test.
fn sanitize_cache_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
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

fn run_license(own: bool) -> io::Result<()> {
    let text = if own {
        include_str!("../LICENSE")
    } else {
        include_str!("../THIRDPARTY-LICENSES.md")
    };
    print!("{text}");
    Ok(())
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
    use std::path::Path;

    use super::{CatalogTarget, DashboardSource, TrustStore, WidgetConfig, allow_render};

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        restore: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(pairs: Vec<(&'static str, Option<String>)>) -> Self {
            let lock = splashboard::paths::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let restore = pairs
                .into_iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    match value {
                        Some(value) => unsafe { std::env::set_var(key, value) },
                        None => unsafe { std::env::remove_var(key) },
                    }
                    (key, previous)
                })
                .collect();
            Self {
                _lock: lock,
                restore,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.restore.iter().for_each(|(key, value)| match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            });
        }
    }

    fn env_with(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    fn minimal_dashboard() -> &'static str {
        r#"
[[widget]]
id = "x"
fetcher = "basic_static"
render = "text_plain"

[[row]]
height = { length = 3 }
[[row.child]]
widget = "x"
"#
    }

    fn write_file(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
    }

    fn widget(id: &str, fetcher: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: fetcher.into(),
            render: None,
            format: None,
            refresh_interval: None,
            file_format: None,
            options: None,
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

    #[test]
    fn run_async_and_render_helpers_return_early_when_disabled() {
        let _env = EnvGuard::set(vec![("TERM", Some("dumb".into()))]);
        let result = super::run_async(async {
            super::render_splash(false).await?;
            super::render_for_cd(false).await
        });
        assert!(result.is_ok());
    }

    #[test]
    fn load_dashboard_file_or_uses_baked_when_path_is_none() {
        let cfg =
            super::load_dashboard_file_or(None, super::DashboardConfig::default_home).unwrap();
        assert!(cfg.widgets.is_empty());
        assert!(cfg.rows.is_empty());
    }

    #[test]
    fn load_dashboard_file_or_uses_baked_when_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.toml");
        let cfg =
            super::load_dashboard_file_or(Some(path), super::DashboardConfig::default_project)
                .unwrap();
        assert!(cfg.widgets.is_empty());
        assert!(cfg.rows.is_empty());
    }

    #[test]
    fn load_dashboard_file_or_reads_valid_dashboard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dashboard.toml");
        write_file(&path, minimal_dashboard());
        let cfg =
            super::load_dashboard_file_or(Some(path), super::DashboardConfig::default_project)
                .unwrap();
        assert_eq!(cfg.widgets.len(), 1);
        assert_eq!(cfg.widgets[0].id, "x");
        assert_eq!(cfg.rows.len(), 1);
    }

    #[test]
    fn load_dashboard_file_or_prefixes_parse_errors_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.toml");
        write_file(&path, "not = [valid");
        let err =
            super::load_dashboard_file_or(Some(path.clone()), super::DashboardConfig::default_home)
                .unwrap_err();
        assert!(err.to_string().contains(&path.display().to_string()));
    }

    #[test]
    fn load_dashboard_file_or_propagates_non_missing_io_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = super::load_dashboard_file_or(
            Some(dir.path().to_path_buf()),
            super::DashboardConfig::default_home,
        )
        .unwrap_err();
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn load_full_config_local_reads_settings_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(vec![(
            "SPLASHBOARD_HOME",
            Some(dir.path().display().to_string()),
        )]);
        write_file(
            &dir.path().join("settings.toml"),
            "[general]\nheight = 23\nwait_for_fresh = true\n",
        );
        let local = dir.path().join("local.dashboard.toml");
        write_file(&local, minimal_dashboard());
        let (config, ident) =
            super::load_full_config(&DashboardSource::Local(local.clone())).unwrap();
        let (path, hash) = ident.expect("local dashboard should include trust identity");
        assert_eq!(path, local);
        assert_eq!(hash.len(), 64);
        assert_eq!(config.general.height, Some(23));
        assert!(config.general.wait_for_fresh);
        assert_eq!(config.widgets[0].id, "x");
    }

    #[test]
    fn load_full_config_home_and_project_use_baked_dashboards_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(vec![(
            "SPLASHBOARD_HOME",
            Some(dir.path().display().to_string()),
        )]);
        let (home, home_ident) = super::load_full_config(&DashboardSource::Home).unwrap();
        let (project, project_ident) = super::load_full_config(&DashboardSource::Project).unwrap();
        assert!(home_ident.is_none());
        assert!(project_ident.is_none());
        assert!(home.widgets.is_empty());
        assert!(project.widgets.is_empty());
    }

    #[test]
    fn apply_secrets_sets_missing_keys_without_overriding_existing_env() {
        let dir = tempfile::tempdir().unwrap();
        let original_path = std::env::var("PATH").unwrap();
        let _env = EnvGuard::set(vec![
            ("SPLASHBOARD_HOME", Some(dir.path().display().to_string())),
            ("MAIN_TEST_SECRET_EXISTING", Some("from_env".to_string())),
            ("MAIN_TEST_SECRET_NEW", None),
        ]);
        write_file(
            &dir.path().join("secrets.toml"),
            r#"
MAIN_TEST_SECRET_EXISTING = "from_file"
MAIN_TEST_SECRET_NEW = "from_file"
PATH = "/tmp/ignored"
"#,
        );
        super::apply_secrets();
        assert_eq!(
            std::env::var("MAIN_TEST_SECRET_EXISTING").ok().as_deref(),
            Some("from_env")
        );
        assert_eq!(
            std::env::var("MAIN_TEST_SECRET_NEW").ok().as_deref(),
            Some("from_file")
        );
        assert_eq!(std::env::var("PATH").unwrap(), original_path);
    }

    #[test]
    fn apply_secrets_ignores_invalid_file() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(vec![
            ("SPLASHBOARD_HOME", Some(dir.path().display().to_string())),
            ("MAIN_TEST_SECRET_BROKEN", None),
        ]);
        write_file(&dir.path().join("secrets.toml"), "not = valid = toml");
        super::apply_secrets();
        assert!(std::env::var("MAIN_TEST_SECRET_BROKEN").is_err());
    }

    #[test]
    fn run_catalog_accepts_successful_targets() {
        assert!(super::run_catalog(None).is_ok());
        assert!(super::run_catalog(Some(CatalogTarget::Fetcher { name: None })).is_ok());
        assert!(
            super::run_catalog(Some(CatalogTarget::Fetcher {
                name: Some("clock".into()),
            }))
            .is_ok()
        );
        assert!(super::run_catalog(Some(CatalogTarget::Renderer { name: None })).is_ok());
        assert!(
            super::run_catalog(Some(CatalogTarget::Renderer {
                name: Some("text_plain".into()),
            }))
            .is_ok()
        );
    }

    #[test]
    fn run_license_prints_both_embedded_bundles() {
        assert!(super::run_license(true).is_ok());
        assert!(super::run_license(false).is_ok());
    }

    #[test]
    fn resolve_trust_target_prefers_override_path() {
        let override_path = Path::new("/tmp/override-dashboard.toml").to_path_buf();
        assert_eq!(
            super::resolve_trust_target(Some(override_path.clone())),
            Some(override_path)
        );
    }

    #[test]
    fn print_trust_summary_handles_network_widgets() {
        let registry = super::Registry::with_builtins();
        let widgets = vec![
            widget("static", "basic_static"),
            widget("feed", "rss"),
            widget("missing", "missing_fetcher"),
        ];
        assert!(
            super::print_trust_summary(Path::new("demo.toml"), "abc123", &widgets, &registry)
                .is_ok()
        );
    }

    #[test]
    fn print_trust_summary_handles_safe_only_widgets() {
        let registry = super::Registry::with_builtins();
        let widgets = vec![widget("static", "basic_static")];
        assert!(
            super::print_trust_summary(Path::new("demo.toml"), "abc123", &widgets, &registry)
                .is_ok()
        );
    }

    #[test]
    fn sanitize_cache_key_matches_alnum_dash_underscore_passthrough() {
        // Keys produced by `fetcher::default_cache_key` look like `<name>-<hex8>` (alnum + dash).
        // They must round-trip through our sanitize unchanged, so the filename we compute in
        // `clear_one` matches what the writer in `cache::Cache::store` chose.
        let key = "clock-3f2a1c8b";
        assert_eq!(super::sanitize_cache_key(key), key);
    }

    #[test]
    fn sanitize_cache_key_replaces_path_and_control_chars() {
        // Defensive: anything outside [A-Za-z0-9_-] must be replaced so the on-disk filename
        // can't escape the cache directory.
        let key = "evil/../name with spaces";
        let sanitized = super::sanitize_cache_key(key);
        assert!(!sanitized.contains('/'));
        assert!(!sanitized.contains('.'));
        assert!(!sanitized.contains(' '));
    }

    #[test]
    fn clear_one_matches_cache_filename_layout() {
        // Pin the contract that `clear_one` uses to find on-disk files: the path produced by
        // `cache::Cache::path_for` for a given key must equal `<dir>/<sanitize_cache_key(key)>.json`.
        // If this test breaks, `cache::sanitize` and `main::sanitize_cache_key` have drifted —
        // update both before merging.
        let dir = tempfile::tempdir().unwrap();
        let cache = splashboard::cache::Cache::open(dir.path().to_path_buf()).unwrap();
        let key = "clock-3f2a1c8b";
        let cache_path = cache.path_for(key);
        let expected = dir
            .path()
            .join(format!("{}.json", super::sanitize_cache_key(key)));
        assert_eq!(cache_path, expected);
    }

    #[test]
    fn collect_cache_rows_returns_empty_for_missing_dir() {
        // `cache list` against a never-used cache should print "(empty)", not error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("never-created");
        let rows = super::collect_cache_rows(&path).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn collect_cache_rows_reads_entry_and_lock_files() {
        use splashboard::cache::{Cache, CacheEntry};
        use splashboard::payload::{Body, Payload, TextData};

        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let payload = Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: "hi".into() }),
        };
        cache
            .store("clock-abc12345", &CacheEntry::new(payload, 60))
            .unwrap();
        // Drop a stray .lock file so we can confirm it's surfaced as an "aux" row.
        std::fs::write(dir.path().join("clock-abc12345.lock"), "").unwrap();
        // And an unrelated file extension that should be skipped silently.
        std::fs::write(dir.path().join("ignored.tmp"), "").unwrap();

        let rows = super::collect_cache_rows(dir.path()).unwrap();
        assert_eq!(rows.len(), 2, "entry + lock should both be returned");
        let entry = rows.iter().find(|r| r.kind == "entry").unwrap();
        assert_eq!(entry.ttl_seconds, 60);
        assert!(entry.freshness == "fresh" || entry.freshness == "stale");
        assert_eq!(entry.outcome, "ok");
        let lock = rows.iter().find(|r| r.kind == "lock").unwrap();
        assert_eq!(lock.ttl_seconds, 0);
        assert_eq!(lock.freshness, "n/a");
    }

    #[test]
    fn clear_all_removes_json_and_lock_files() {
        let dir = tempfile::tempdir().unwrap();
        // Seed: 1 entry + 1 lock + 1 unrelated file. Only the first two should be removed.
        std::fs::write(dir.path().join("x.json"), r#"{}"#).unwrap();
        std::fs::write(dir.path().join("x.lock"), "").unwrap();
        std::fs::write(dir.path().join("note.txt"), "preserve me").unwrap();

        super::clear_all(dir.path(), /* yes = */ true, /* json = */ false).unwrap();

        assert!(!dir.path().join("x.json").exists());
        assert!(!dir.path().join("x.lock").exists());
        assert!(
            dir.path().join("note.txt").exists(),
            "non-cache files must not be touched"
        );
    }

    #[test]
    fn clear_all_handles_missing_dir_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("never-created");
        super::clear_all(&path, /* yes = */ true, /* json = */ false).unwrap();
    }

    #[test]
    fn run_list_trusted_and_revoke_round_trip_store_entries() {
        let dir = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set(vec![(
            "SPLASHBOARD_HOME",
            Some(dir.path().display().to_string()),
        )]);
        let dashboard = dir.path().join("trusted.dashboard.toml");
        write_file(&dashboard, minimal_dashboard());
        let mut store = TrustStore::load();
        store.trust(&dashboard, "abc123".into()).unwrap();
        assert!(super::run_list_trusted().is_ok());
        assert!(super::run_revoke(Some(dashboard.clone())).is_ok());
        assert!(TrustStore::load().list().is_empty());
        assert!(super::run_revoke(Some(dashboard)).is_ok());
    }
}
