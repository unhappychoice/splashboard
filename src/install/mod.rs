//! One-shot onboarding: `splashboard install` wires the shell rc + writes a starter
//! home/project dashboard pair from the template catalog. In a TTY we open a small
//! gallery picker so users preview each preset before committing; non-TTY callers must
//! supply `--home-template` / `--project-template` for a deterministic scripted flow.

mod picker;
mod preview;
mod rc;

use std::io::{self, IsTerminal, stdout};
use std::path::{Path, PathBuf};

use crate::paths;
use crate::shell::{self, Shell};
use crate::templates::{self, Template, TemplateContext};
use crate::theme::presets as theme_presets;

/// Inputs parsed from the CLI. All optional — the installer fills gaps interactively
/// when a TTY is available, or fails with an actionable message otherwise.
#[derive(Debug, Default, Clone)]
pub struct InstallOptions {
    pub shell: Option<Shell>,
    pub home_template: Option<String>,
    pub project_template: Option<String>,
    /// Theme preset name from [`crate::theme::presets::KNOWN`]. `None` leaves `settings.toml`
    /// without a `preset = "..."` line (which selects the built-in Splash palette).
    pub theme: Option<String>,
    /// Whether the splash paints its own background. `Some(false)` writes
    /// `bg = "reset"` + `bg_subtle = "reset"` so the terminal's own bg shows through
    /// (useful on light terminals or transparent backgrounds). `None` inherits `true`
    /// as the default.
    pub bg: Option<bool>,
    /// Whether to block the initial paint until every widget has fetched fresh data.
    /// `Some(true)` writes `wait_for_fresh = true`. `None` inherits `false`.
    pub wait: Option<bool>,
    /// Overrides both the home and project dashboard destinations with this explicit
    /// directory. Primarily for tests; users should lean on `SPLASHBOARD_HOME`.
    pub config_dir: Option<PathBuf>,
    pub rc_path: Option<PathBuf>,
}

/// Resolved toggle values used during install. Separated from [`InstallOptions`] so the
/// post-picker flow can carry concrete values without re-threading the `Option`s.
#[derive(Debug, Clone, Copy)]
struct SettingsChoices {
    theme: Option<&'static str>,
    bg: bool,
    wait: bool,
}

impl SettingsChoices {
    const DEFAULT: Self = Self {
        theme: None,
        bg: true,
        wait: false,
    };
}

pub fn run(opts: InstallOptions) -> io::Result<()> {
    let shell = resolve_shell(opts.shell)?;
    let home_dir = resolve_home_dir(opts.config_dir.clone())?;
    let destinations = Destinations::for_home_dir(&home_dir);

    // Decide up front what the picker needs to ask. Each axis is independent so a user
    // who passes `--home-template` / `--project-template` but not `--theme` still gets
    // the theme + toggle screens. The settings picker runs whenever the user isn't being
    // explicit about theme/bg/wait (via flags) — each file write is inherently safe
    // because the old content lands in a `.bak` sidecar before the new content is written.
    let need_templates = opts.home_template.is_none() || opts.project_template.is_none();
    let need_settings_picker = opts.theme.is_none() && opts.bg.is_none() && opts.wait.is_none();

    let theme_from_flag = match opts.theme.as_deref() {
        Some(name) => Some(Some(validate_theme(name)?)),
        None => None,
    };

    let picker_ran = is_tty() && (need_templates || need_settings_picker);
    let (home, project, settings_choices) = if picker_ran {
        let pre_home = match opts.home_template.as_deref() {
            Some(name) => Some(select_template(Some(name), TemplateContext::Home)?),
            None => None,
        };
        let needs = picker::Needs {
            templates: need_templates,
            settings: need_settings_picker,
        };
        let Some(selection) = picker::run(needs, pre_home)? else {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "install cancelled",
            ));
        };
        let home = match selection.templates.as_ref() {
            Some(p) => p.home,
            None => select_template(opts.home_template.as_deref(), TemplateContext::Home)?,
        };
        let project = match selection.templates.as_ref() {
            Some(p) => p.project,
            None => select_template(opts.project_template.as_deref(), TemplateContext::Project)?,
        };
        let settings = selection
            .settings
            .unwrap_or_else(|| merge_settings_flags(&opts, theme_from_flag));
        (home, project, settings)
    } else {
        let home = select_template(opts.home_template.as_deref(), TemplateContext::Home)?;
        let project = select_template(opts.project_template.as_deref(), TemplateContext::Project)?;
        let settings = merge_settings_flags(&opts, theme_from_flag);
        (home, project, settings)
    };

    // Confirmation page — only shown when the user went through the picker. A scripted
    // invocation with explicit flags is its own confirmation; forcing a TTY prompt would
    // break dotfile bootstrap.
    if picker_ran {
        let rc_path = opts
            .rc_path
            .clone()
            .or_else(|| shell::default_rc_path(shell));
        let summary = picker::Summary {
            shell: shell.as_str().to_string(),
            home_template: home.name.to_string(),
            project_template: project.name.to_string(),
            theme: settings_choices.theme.unwrap_or("default").to_string(),
            bg: settings_choices.bg,
            wait: settings_choices.wait,
            files: vec![
                destinations.home_dashboard.clone(),
                destinations.project_dashboard.clone(),
                destinations.settings.clone(),
            ],
            rc_path,
        };
        if !picker::confirm(summary)? {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "install cancelled",
            ));
        }
    }

    ensure_parent_dirs(&destinations)?;
    ensure_secrets_gitignore(&home_dir)?;
    let mut write_report = write_dashboards(&destinations, home, project)?;
    let settings_report = write_settings(
        &destinations.settings,
        settings_choices,
        &mut write_report.backups,
    )?;
    let rc_report = rc::wire_shell_rc(shell, opts.rc_path.clone())?;

    print_summary(SummaryContext {
        shell,
        home,
        project,
        dest: &destinations,
        write: &write_report,
        settings: &settings_report,
        rc: &rc_report,
        settings_choices,
    });
    Ok(())
}

fn resolve_shell(override_shell: Option<Shell>) -> io::Result<Shell> {
    if let Some(shell) = override_shell {
        return Ok(shell);
    }
    if let Some(shell) = shell::detect_shell(|k| std::env::var(k).ok()) {
        return Ok(shell);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "could not detect your shell — pass --shell <bash|zsh|fish|powershell>",
    ))
}

/// Combines explicit flags with defaults for the scripted / non-TTY flow. `theme_from_flag`
/// is the validated theme key (double-`Option`: outer `None` = no flag, inner `None` = flag
/// set to "default"). Callers that use the picker skip this and consume the picker output
/// instead.
fn merge_settings_flags(
    opts: &InstallOptions,
    theme_from_flag: Option<Option<&'static str>>,
) -> SettingsChoices {
    SettingsChoices {
        theme: theme_from_flag.unwrap_or(None),
        bg: opts.bg.unwrap_or(SettingsChoices::DEFAULT.bg),
        wait: opts.wait.unwrap_or(SettingsChoices::DEFAULT.wait),
    }
}

fn validate_theme(name: &str) -> io::Result<&'static str> {
    theme_presets::KNOWN
        .iter()
        .copied()
        .find(|k| *k == name)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unknown theme `{name}` (available: {})",
                    theme_presets::KNOWN.join(", ")
                ),
            )
        })
}

fn select_template(name: Option<&str>, context: TemplateContext) -> io::Result<&'static Template> {
    let Some(name) = name else {
        let label = match context {
            TemplateContext::Home => "--home-template",
            TemplateContext::Project => "--project-template",
        };
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{label} is required in non-interactive mode ({} available)",
                catalog_hint(context)
            ),
        ));
    };
    match templates::find(name) {
        Some(t) if t.context == context => Ok(t),
        Some(t) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "template `{name}` is a {:?} template; expected a {:?} template",
                t.context, context
            ),
        )),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unknown template `{name}` (available: {})",
                catalog_hint(context)
            ),
        )),
    }
}

fn catalog_hint(context: TemplateContext) -> String {
    templates::for_context(context)
        .map(|t| t.name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_home_dir(override_dir: Option<PathBuf>) -> io::Result<PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    paths::splashboard_home().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not locate $HOME — set $HOME or $SPLASHBOARD_HOME",
        )
    })
}

fn is_tty() -> bool {
    stdout().is_terminal() && io::stdin().is_terminal()
}

struct Destinations {
    settings: PathBuf,
    home_dashboard: PathBuf,
    project_dashboard: PathBuf,
}

impl Destinations {
    fn for_home_dir(dir: &Path) -> Self {
        Self {
            settings: dir.join("settings.toml"),
            home_dashboard: dir.join("home.dashboard.toml"),
            project_dashboard: dir.join("project.dashboard.toml"),
        }
    }
}

/// Drops `secrets.toml` into `$HOME/.splashboard/.gitignore` so dotfiles-in-git users who
/// commit the rest of `.splashboard/` don't leak their tokens by accident. Idempotent:
/// appends the line only when missing, leaves the rest of the file alone.
fn ensure_secrets_gitignore(home_dir: &Path) -> io::Result<()> {
    let path = home_dir.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.trim() == "secrets.toml" || l.trim() == "/secrets.toml")
    {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if next.is_empty() {
        next.push_str("# Written by `splashboard install` — keeps tokens out of dotfiles repos.\n");
    }
    next.push_str("secrets.toml\n");
    std::fs::write(&path, next)
}

fn ensure_parent_dirs(dest: &Destinations) -> io::Result<()> {
    for path in [
        &dest.settings,
        &dest.home_dashboard,
        &dest.project_dashboard,
    ] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteOutcome {
    Created,
    Overwrote,
    /// Existing file already matches the new content — nothing written, nothing backed up.
    /// Lets a fresh re-run (e.g. `splashboard install` with the same flags as last time)
    /// stay noise-free in the summary instead of churning a `.bak` sidecar needlessly.
    Unchanged,
}

struct DashboardWriteReport {
    home: WriteOutcome,
    project: WriteOutcome,
    backups: Vec<PathBuf>,
}

fn write_dashboards(
    dest: &Destinations,
    home: &Template,
    project: &Template,
) -> io::Result<DashboardWriteReport> {
    let mut backups = Vec::new();
    let home_outcome = write_template(&dest.home_dashboard, home.body, &mut backups)?;
    let project_outcome = write_template(&dest.project_dashboard, project.body, &mut backups)?;
    Ok(DashboardWriteReport {
        home: home_outcome,
        project: project_outcome,
        backups,
    })
}

/// Writes `body` to `path`, renaming any existing file to a `.bak` sidecar first. Skips
/// the rename / write entirely when the existing content already matches `body` — that
/// keeps re-running the installer with the same flags from churning noise into `.bak`.
/// The old content is always recoverable from the sidecar, so the installer never asks
/// "are you sure?" before overwriting — `.bak` _is_ the confirmation step.
fn write_template(path: &Path, body: &str, backups: &mut Vec<PathBuf>) -> io::Result<WriteOutcome> {
    if path.exists() {
        if std::fs::read_to_string(path).ok().as_deref() == Some(body) {
            return Ok(WriteOutcome::Unchanged);
        }
        let backup = path.with_extension(backup_extension(path));
        std::fs::rename(path, &backup)?;
        backups.push(backup);
        std::fs::write(path, body)?;
        return Ok(WriteOutcome::Overwrote);
    }
    std::fs::write(path, body)?;
    Ok(WriteOutcome::Created)
}

fn backup_extension(path: &Path) -> String {
    // `home.dashboard.toml` → `home.dashboard.toml.bak`. We can't just call
    // `set_extension("bak")` because that replaces `.toml` and we'd end up with
    // `home.dashboard.bak`, which collides with a different conceptual file.
    format!(
        "{}.bak",
        path.extension().and_then(|e| e.to_str()).unwrap_or("bak")
    )
}

/// Writes / refreshes `settings.toml`. Follows the same `.bak`-sidecar model as
/// dashboards — existing files are renamed out of the way before the new content lands,
/// and a no-op re-run (same flags as last time) short-circuits to `Unchanged` so the
/// summary and `.bak` stash don't churn.
fn write_settings(
    path: &Path,
    choices: SettingsChoices,
    backups: &mut Vec<PathBuf>,
) -> io::Result<WriteOutcome> {
    let body = build_settings_toml(choices);
    write_template(path, &body, backups)
}

/// Emits a settings.toml body from the chosen toggles. Omits every key that matches the
/// built-in default so a user who picks the defaults ends up with a near-empty file they
/// can extend rather than one full of no-op overrides.
fn build_settings_toml(choices: SettingsChoices) -> String {
    let mut out = String::new();
    out.push_str(
        "# Written by `splashboard install`. User preferences that apply to every dashboard.\n\
         # Safe to edit by hand. Re-running `splashboard install` leaves this file alone\n\
         # unless you pass `--yes`, which backs it up as `settings.toml.bak` before rewriting.\n\n",
    );

    let mut wrote_general = false;
    if choices.wait != SettingsChoices::DEFAULT.wait {
        out.push_str("[general]\n");
        out.push_str(&format!("wait_for_fresh = {}\n\n", choices.wait));
        wrote_general = true;
    }
    let needs_theme_section = choices.theme.is_some() || !choices.bg;
    if needs_theme_section {
        if !wrote_general {
            // Otherwise the only section boundary below is the theme block.
        }
        out.push_str("[theme]\n");
        if let Some(name) = choices.theme {
            out.push_str(&format!("preset = \"{name}\"\n"));
        }
        if !choices.bg {
            // `"reset"` lets the terminal's own background show through instead of painting
            // the Splash palette's navy. We set both `bg` and `bg_subtle` so subtle panels
            // stay transparent too — otherwise `bg = "subtle"` widgets would leak a tinted
            // background back in.
            out.push_str("bg = \"reset\"\n");
            out.push_str("bg_subtle = \"reset\"\n");
        }
        out.push('\n');
    }
    out
}

struct SummaryContext<'a> {
    shell: Shell,
    home: &'a Template,
    project: &'a Template,
    dest: &'a Destinations,
    write: &'a DashboardWriteReport,
    settings: &'a WriteOutcome,
    rc: &'a rc::RcReport,
    settings_choices: SettingsChoices,
}

fn print_summary(ctx: SummaryContext<'_>) {
    let SummaryContext {
        shell,
        home,
        project,
        dest,
        write,
        settings,
        rc,
        settings_choices,
    } = ctx;
    println!("splashboard install — done.");
    println!();
    println!("Shell:     {}", shell.as_str());
    println!(
        "Templates: {} (home) · {} (project)",
        home.name, project.name
    );
    println!(
        "Settings:  theme = {} · bg = {} · wait_for_fresh = {}",
        settings_choices.theme.unwrap_or("default"),
        if settings_choices.bg {
            "on"
        } else {
            "off (terminal bg shows through)"
        },
        settings_choices.wait,
    );
    println!();
    println!("Files:");
    println!(
        "  {:8} {}",
        label_for_outcome(write.home),
        dest.home_dashboard.display()
    );
    println!(
        "  {:8} {}",
        label_for_outcome(write.project),
        dest.project_dashboard.display()
    );
    println!(
        "  {:8} {}",
        label_for_outcome(*settings),
        dest.settings.display()
    );
    for b in &write.backups {
        println!("  backup   {}", b.display());
    }
    rc.describe();
    println!();
    println!("Next steps:");
    println!(
        "  • Start a new shell (or `source {}`) to see the splash.",
        rc.rc_path_display()
    );
    println!("  • Browse widgets: `splashboard catalog`.");
    println!(
        "  • Re-run with a different preset: `splashboard install --home-template <name> --project-template <name>` (old files land in `.bak`)."
    );
}

fn label_for_outcome(outcome: WriteOutcome) -> &'static str {
    match outcome {
        WriteOutcome::Created => "created",
        WriteOutcome::Overwrote => "updated",
        WriteOutcome::Unchanged => "unchanged",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn select_template_rejects_context_mismatch() {
        let err = select_template(Some("home_splash"), TemplateContext::Project).unwrap_err();
        assert!(err.to_string().contains("Home template"));
    }

    #[test]
    fn select_template_accepts_matching_context() {
        let t = select_template(Some("home_splash"), TemplateContext::Home).unwrap();
        assert_eq!(t.name, "home_splash");
    }

    #[test]
    fn select_template_rejects_unknown_name() {
        let err = select_template(Some("nonsense"), TemplateContext::Home).unwrap_err();
        assert!(err.to_string().contains("unknown template"));
    }

    #[test]
    fn select_template_requires_name_for_both_contexts() {
        let home = select_template(None, TemplateContext::Home).unwrap_err();
        assert!(home.to_string().contains("--home-template"));
        let project = select_template(None, TemplateContext::Project).unwrap_err();
        assert!(project.to_string().contains("--project-template"));
    }

    #[test]
    fn merge_settings_flags_preserves_explicit_and_default_values() {
        let explicit = merge_settings_flags(
            &InstallOptions {
                bg: Some(false),
                wait: Some(true),
                ..Default::default()
            },
            Some(Some("tokyo_night")),
        );
        assert_eq!(explicit.theme, Some("tokyo_night"));
        assert!(!explicit.bg);
        assert!(explicit.wait);

        let default_theme = merge_settings_flags(&InstallOptions::default(), Some(None));
        assert_eq!(default_theme.theme, None);
        assert!(default_theme.bg);
        assert!(!default_theme.wait);
    }

    #[test]
    fn catalog_hint_lists_templates_for_requested_context() {
        let home = catalog_hint(TemplateContext::Home);
        assert!(home.contains("home_splash"));
        assert!(!home.contains("project_github"));

        let project = catalog_hint(TemplateContext::Project);
        assert!(project.contains("project_github"));
        assert!(!project.contains("home_splash"));
    }

    #[test]
    fn destinations_and_parent_dirs_use_expected_paths() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("nested").join("splashboard");
        let dest = Destinations::for_home_dir(&home);
        assert_eq!(dest.settings, home.join("settings.toml"));
        assert_eq!(dest.home_dashboard, home.join("home.dashboard.toml"));
        assert_eq!(dest.project_dashboard, home.join("project.dashboard.toml"));
        ensure_parent_dirs(&dest).unwrap();
        assert!(home.exists());
    }

    #[test]
    fn write_dashboards_creates_missing_files() {
        let dir = tempdir().unwrap();
        let dest = Destinations::for_home_dir(dir.path());
        let home = templates::find("home_splash").unwrap();
        let project = templates::find("project_github").unwrap();
        let report = write_dashboards(&dest, home, project).unwrap();
        assert_eq!(report.home, WriteOutcome::Created);
        assert_eq!(report.project, WriteOutcome::Created);
        assert!(dest.home_dashboard.exists());
        assert!(dest.project_dashboard.exists());
        assert!(report.backups.is_empty());
    }

    #[test]
    fn write_dashboards_overwrites_with_backup() {
        let dir = tempdir().unwrap();
        let dest = Destinations::for_home_dir(dir.path());
        std::fs::write(&dest.home_dashboard, "# existing\n").unwrap();
        let home = templates::find("home_splash").unwrap();
        let project = templates::find("project_github").unwrap();
        let report = write_dashboards(&dest, home, project).unwrap();
        assert_eq!(report.home, WriteOutcome::Overwrote);
        assert_eq!(report.project, WriteOutcome::Created);
        assert_eq!(report.backups.len(), 1);
        assert!(report.backups[0].exists());
        assert_eq!(
            std::fs::read_to_string(&report.backups[0]).unwrap(),
            "# existing\n"
        );
    }

    /// Second run with the same templates is a no-op — no backup churn, file stays as
    /// `Unchanged`. Important so users who re-run `install` (e.g. to pick up a new
    /// shell) don't accumulate pointless `.bak` sidecars.
    #[test]
    fn write_dashboards_is_noop_when_content_unchanged() {
        let dir = tempdir().unwrap();
        let dest = Destinations::for_home_dir(dir.path());
        let home = templates::find("home_splash").unwrap();
        let project = templates::find("project_github").unwrap();
        // First run populates.
        write_dashboards(&dest, home, project).unwrap();
        // Second run with the same inputs should short-circuit.
        let report = write_dashboards(&dest, home, project).unwrap();
        assert_eq!(report.home, WriteOutcome::Unchanged);
        assert_eq!(report.project, WriteOutcome::Unchanged);
        assert!(report.backups.is_empty());
    }

    #[test]
    fn write_settings_creates_when_missing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut backups = Vec::new();
        assert_eq!(
            write_settings(&path, SettingsChoices::DEFAULT, &mut backups).unwrap(),
            WriteOutcome::Created
        );
        assert!(backups.is_empty());
    }

    #[test]
    fn write_settings_backs_up_and_overwrites() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "# prior settings\n").unwrap();
        let mut backups = Vec::new();
        let choices = SettingsChoices {
            theme: Some("tokyo_night"),
            bg: false,
            wait: true,
        };
        assert_eq!(
            write_settings(&path, choices, &mut backups).unwrap(),
            WriteOutcome::Overwrote
        );
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&backups[0]).unwrap(),
            "# prior settings\n"
        );
        let new_contents = std::fs::read_to_string(&path).unwrap();
        assert!(new_contents.contains("preset = \"tokyo_night\""));
        assert!(new_contents.contains("wait_for_fresh = true"));
    }

    /// Re-running settings with the same choices shouldn't churn a `.bak` sidecar.
    #[test]
    fn write_settings_is_noop_when_content_unchanged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let choices = SettingsChoices {
            theme: Some("tokyo_night"),
            bg: true,
            wait: false,
        };
        let mut backups = Vec::new();
        write_settings(&path, choices, &mut backups).unwrap();
        let report = write_settings(&path, choices, &mut backups).unwrap();
        assert_eq!(report, WriteOutcome::Unchanged);
        assert!(backups.is_empty());
    }

    #[test]
    fn build_settings_toml_emits_empty_for_defaults() {
        let body = build_settings_toml(SettingsChoices::DEFAULT);
        // Defaults → no `[general]` or `[theme]` sections, just the leading preamble comment.
        assert!(!body.contains("[general]"));
        assert!(!body.contains("[theme]"));
        assert!(body.contains("Written by `splashboard install`"));
    }

    #[test]
    fn build_settings_toml_writes_theme_and_bg_reset() {
        let body = build_settings_toml(SettingsChoices {
            theme: Some("tokyo_night"),
            bg: false,
            wait: false,
        });
        assert!(body.contains("[theme]"));
        assert!(body.contains("preset = \"tokyo_night\""));
        assert!(body.contains("bg = \"reset\""));
        assert!(body.contains("bg_subtle = \"reset\""));
        assert!(!body.contains("[general]"));
    }

    #[test]
    fn build_settings_toml_writes_wait_for_fresh() {
        let body = build_settings_toml(SettingsChoices {
            theme: None,
            bg: true,
            wait: true,
        });
        assert!(body.contains("[general]"));
        assert!(body.contains("wait_for_fresh = true"));
        assert!(!body.contains("[theme]"));
    }

    #[test]
    fn validate_theme_accepts_known_preset() {
        assert_eq!(validate_theme("tokyo_night").unwrap(), "tokyo_night");
    }

    #[test]
    fn validate_theme_rejects_unknown_preset() {
        let err = validate_theme("not-a-theme").unwrap_err();
        assert!(err.to_string().contains("unknown theme"));
    }

    /// End-to-end cut of `install::run()`: exercises shell resolution, template lookup,
    /// dashboard writes, the settings-default guard and rc wiring together to catch the
    /// kind of orchestration bugs the per-function tests miss.
    #[test]
    fn run_end_to_end_wires_everything_on_fresh_home() {
        let dir = tempdir().unwrap();
        let rc = dir.path().join(".zshrc");
        let home_dir = dir.path().join("splashboard_home");
        let opts = InstallOptions {
            shell: Some(Shell::Zsh),
            home_template: Some("home_splash".into()),
            project_template: Some("project_github".into()),
            config_dir: Some(home_dir.clone()),
            rc_path: Some(rc.clone()),
            ..Default::default()
        };
        super::run(opts).unwrap();
        assert!(home_dir.join("home.dashboard.toml").exists());
        assert!(home_dir.join("project.dashboard.toml").exists());
        assert!(home_dir.join("settings.toml").exists());
        assert!(rc.exists());
        let rc_contents = std::fs::read_to_string(&rc).unwrap();
        assert!(rc_contents.contains("splashboard init zsh"));
    }

    /// `--theme` / `--no-bg` / `--wait` flags on a fresh install land in the generated
    /// `settings.toml`.
    #[test]
    fn run_end_to_end_applies_settings_flags() {
        let dir = tempdir().unwrap();
        let rc = dir.path().join(".zshrc");
        let home_dir = dir.path().join("splashboard_home");
        let opts = InstallOptions {
            shell: Some(Shell::Zsh),
            home_template: Some("home_splash".into()),
            project_template: Some("project_github".into()),
            theme: Some("tokyo_night".into()),
            bg: Some(false),
            wait: Some(true),
            config_dir: Some(home_dir.clone()),
            rc_path: Some(rc.clone()),
        };
        super::run(opts).unwrap();
        let settings = std::fs::read_to_string(home_dir.join("settings.toml")).unwrap();
        assert!(settings.contains("preset = \"tokyo_night\""));
        assert!(settings.contains("bg = \"reset\""));
        assert!(settings.contains("wait_for_fresh = true"));
    }

    /// Re-running with different settings flags backs the prior `settings.toml` up to
    /// `.bak` and writes the new content. No `--yes` gate — the `.bak` sidecar is the
    /// safety net, and users who want the old state back just rename it.
    #[test]
    fn run_end_to_end_regenerates_settings_on_rerun() {
        let dir = tempdir().unwrap();
        let rc = dir.path().join(".zshrc");
        let home_dir = dir.path().join("splashboard_home");
        std::fs::create_dir_all(&home_dir).unwrap();
        let settings_path = home_dir.join("settings.toml");
        std::fs::write(&settings_path, "# hand-edited settings\n").unwrap();

        let opts = InstallOptions {
            shell: Some(Shell::Zsh),
            home_template: Some("home_splash".into()),
            project_template: Some("project_github".into()),
            theme: Some("tokyo_night".into()),
            config_dir: Some(home_dir.clone()),
            rc_path: Some(rc.clone()),
            ..Default::default()
        };
        super::run(opts).unwrap();

        let new_contents = std::fs::read_to_string(&settings_path).unwrap();
        assert!(new_contents.contains("preset = \"tokyo_night\""));
        // `.toml` → `.toml.bak`: make sure the sidecar exists and carries the prior body.
        let backup = home_dir.join("settings.toml.bak");
        assert!(backup.exists(), "expected settings.toml.bak backup");
        assert_eq!(
            std::fs::read_to_string(&backup).unwrap(),
            "# hand-edited settings\n"
        );
    }

    #[test]
    fn ensure_secrets_gitignore_creates_when_missing() {
        let dir = tempdir().unwrap();
        ensure_secrets_gitignore(dir.path()).unwrap();
        let body = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(body.contains("secrets.toml"));
    }

    #[test]
    fn ensure_secrets_gitignore_appends_to_existing() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules\n").unwrap();
        ensure_secrets_gitignore(dir.path()).unwrap();
        let body = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(body.contains("node_modules"));
        assert!(body.contains("secrets.toml"));
    }

    #[test]
    fn ensure_secrets_gitignore_is_idempotent() {
        let dir = tempdir().unwrap();
        ensure_secrets_gitignore(dir.path()).unwrap();
        let first = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        ensure_secrets_gitignore(dir.path()).unwrap();
        let second = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn ensure_secrets_gitignore_treats_slashed_entry_as_already_present() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        std::fs::write(&path, "node_modules\n/secrets.toml").unwrap();
        ensure_secrets_gitignore(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "node_modules\n/secrets.toml"
        );
    }

    #[test]
    fn backup_extension_and_labels_cover_all_variants() {
        assert_eq!(
            backup_extension(Path::new("home.dashboard.toml")),
            "toml.bak"
        );
        assert_eq!(backup_extension(Path::new("settings")), "bak.bak");
        assert_eq!(label_for_outcome(WriteOutcome::Created), "created");
        assert_eq!(label_for_outcome(WriteOutcome::Overwrote), "updated");
        assert_eq!(label_for_outcome(WriteOutcome::Unchanged), "unchanged");
    }

    #[test]
    fn run_rejects_missing_templates_in_non_interactive_mode() {
        let dir = tempdir().unwrap();
        let home_dir = dir.path().join("splashboard_home");
        let home_err = super::run(InstallOptions {
            shell: Some(Shell::Zsh),
            project_template: Some("project_github".into()),
            config_dir: Some(home_dir.clone()),
            rc_path: Some(dir.path().join(".zshrc")),
            ..Default::default()
        })
        .unwrap_err();
        assert!(home_err.to_string().contains("--home-template"));

        let project_err = super::run(InstallOptions {
            shell: Some(Shell::Zsh),
            home_template: Some("home_splash".into()),
            config_dir: Some(home_dir),
            rc_path: Some(dir.path().join(".zshrc")),
            ..Default::default()
        })
        .unwrap_err();
        assert!(project_err.to_string().contains("--project-template"));
    }

    #[test]
    fn run_rejects_unknown_theme_before_writing_files() {
        let dir = tempdir().unwrap();
        let home_dir = dir.path().join("splashboard_home");
        let rc = dir.path().join(".zshrc");
        let err = super::run(InstallOptions {
            shell: Some(Shell::Zsh),
            home_template: Some("home_splash".into()),
            project_template: Some("project_github".into()),
            theme: Some("not-a-theme".into()),
            config_dir: Some(home_dir.clone()),
            rc_path: Some(rc.clone()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.to_string().contains("unknown theme"));
        assert!(!home_dir.exists());
        assert!(!rc.exists());
    }

    /// Running `install` twice with identical flags should be a no-op — no backup
    /// churn, rc block reported as already-wired, dashboards and settings both land
    /// as `Unchanged`.
    #[test]
    fn run_end_to_end_is_idempotent_on_second_run() {
        let dir = tempdir().unwrap();
        let rc = dir.path().join(".zshrc");
        let home_dir = dir.path().join("splashboard_home");
        let opts = InstallOptions {
            shell: Some(Shell::Zsh),
            home_template: Some("home_splash".into()),
            project_template: Some("project_github".into()),
            theme: Some("tokyo_night".into()),
            config_dir: Some(home_dir.clone()),
            rc_path: Some(rc.clone()),
            ..Default::default()
        };
        super::run(opts.clone()).unwrap();
        super::run(opts).unwrap();

        // Second run didn't leave any `.bak` sidecars lying around.
        assert!(!home_dir.join("home.dashboard.toml.bak").exists());
        assert!(!home_dir.join("project.dashboard.toml.bak").exists());
        assert!(!home_dir.join("settings.toml.bak").exists());
        // And the rc still has exactly one block.
        let rc_contents = std::fs::read_to_string(&rc).unwrap();
        assert_eq!(rc_contents.matches("# >>> splashboard >>>").count(), 1);
    }
}
