//! Workspace tooling for splashboard. Run via `cargo xtask`.
//!
//! Reads the built-in fetcher / renderer registries and writes Markdown reference pages (one
//! per fetcher, one per renderer, plus an overview). Also renders full dashboard TOMLs to
//! inline HTML using each fetcher's `sample_body`, so the docs site can show real splash
//! output without ever touching the network.

use std::path::{Path, PathBuf};
use std::{fs, io::Write};

use anyhow::{Context, Result};
use clap::Parser;

mod dashboard_snapshot;
mod gen_matrix;
mod html_snapshot;
mod snapshots;

#[derive(Parser)]
#[command(
    version,
    about = "Generate splashboard's fetcher/renderer reference pages"
)]
struct Cli {
    /// Destination directory. Writes `<out>/matrix.md`, `<out>/fetchers/<name>.md`,
    /// `<out>/renderers/<name>.md`. Default lands inside the Starlight content tree so
    /// `astro build` picks the pages up without further wiring.
    #[arg(long, default_value = "docs-site/src/content/docs/reference")]
    out: PathBuf,

    /// Destination directory for rendered dashboard HTML snippets, imported via `?raw` by
    /// `index.mdx` and other landing surfaces. One file per entry in `DASHBOARD_SNAPSHOTS`.
    #[arg(long, default_value = "docs-site/src/assets/rendered")]
    rendered_out: PathBuf,
}

/// Every preset renders at the same 120 × 42 cell canvas so the embedded snapshots read as a
/// uniform gallery under `.splash-landing` (the CSS scales font-size off a 120-cell baseline and
/// `.splash-snapshot` has no fixed aspect ratio). 42 = project_github's natural height;
/// shorter presets get blank rows of theme bg at the bottom, same as an oversized terminal would.
const SNAPSHOT_WIDTH: u16 = 120;
const SNAPSHOT_HEIGHT: u16 = 42;

const DASHBOARD_SNAPSHOTS: &[(&str, &str)] = &[
    ("src/templates/home_splash.toml", "home_splash.html"),
    ("src/templates/home_daily.toml", "home_daily.html"),
    ("src/templates/home_github.toml", "home_github.html"),
    ("src/templates/home_minimal.toml", "home_minimal.html"),
    ("src/templates/home_feed.toml", "home_feed.html"),
    ("src/templates/project_splash.toml", "project_splash.html"),
    ("src/templates/project_github.toml", "project_github.html"),
    ("src/templates/project_minimal.toml", "project_minimal.html"),
    (
        "src/templates/project_codebase.toml",
        "project_codebase.html",
    ),
];

fn main() -> Result<()> {
    let cli = Cli::parse();
    gen_matrix::run(&cli.out)?;
    render_dashboards(&cli.rendered_out)?;
    Ok(())
}

fn render_dashboards(out_dir: &Path) -> Result<()> {
    // Realtime fetchers (invoked by dashboard_snapshot::sample_payloads) peek at env vars to
    // guess the terminal / shell / host. Scrub the signals that vary by dev machine so the
    // committed snapshot is deterministic — "terminal" / "shell" fallbacks, empty hostname.
    scrub_host_env();
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    for (config, output_name) in DASHBOARD_SNAPSHOTS {
        let html = dashboard_snapshot::render_config_html(
            Path::new(config),
            SNAPSHOT_WIDTH,
            SNAPSHOT_HEIGHT,
        )?;
        let dest = out_dir.join(output_name);
        let mut f =
            fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        f.write_all(html.as_bytes())
            .with_context(|| format!("write {}", dest.display()))?;
        println!("wrote {}", dest.display());
    }
    Ok(())
}

fn scrub_host_env() {
    for key in [
        "WT_SESSION",
        "GHOSTTY_RESOURCES_DIR",
        "KITTY_WINDOW_ID",
        "ALACRITTY_WINDOW_ID",
        "ALACRITTY_LOG",
        "WEZTERM_PANE",
        "TERM_PROGRAM",
    ] {
        // SAFETY: xtask is a single-threaded CLI entry point; no other threads can race this.
        unsafe { std::env::remove_var(key) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        values: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                values: keys
                    .iter()
                    .map(|key| (*key, std::env::var_os(key)))
                    .collect(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            self.values.iter().for_each(|(key, value)| {
                // SAFETY: tests restore the original process env under the shared lock below.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            });
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = unique_path(label);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct CwdGuard {
        previous: PathBuf,
    }

    impl CwdGuard {
        fn change_to(path: &Path) -> Self {
            let previous = std::env::current_dir().unwrap();
            std::env::set_current_dir(path).unwrap();
            Self { previous }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.previous).unwrap();
        }
    }

    fn unique_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "splashboard-xtask-main-{label}-{unique}-{}",
            std::process::id()
        ))
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn cli_parse_uses_documented_default_paths() {
        let cli = Cli::parse_from(["xtask"]);
        assert_eq!(
            cli.out,
            PathBuf::from("docs-site/src/content/docs/reference")
        );
        assert_eq!(
            cli.rendered_out,
            PathBuf::from("docs-site/src/assets/rendered")
        );
    }

    #[test]
    fn cli_parse_accepts_output_overrides() {
        let cli = Cli::parse_from([
            "xtask",
            "--out",
            "tmp/docs",
            "--rendered-out",
            "tmp/rendered",
        ]);
        assert_eq!(cli.out, PathBuf::from("tmp/docs"));
        assert_eq!(cli.rendered_out, PathBuf::from("tmp/rendered"));
    }

    #[test]
    fn unique_path_changes_with_inputs() {
        let alpha = unique_path("alpha");
        let beta = unique_path("beta");
        assert_ne!(alpha, beta);
        assert!(alpha.to_string_lossy().contains("alpha"));
        assert!(beta.to_string_lossy().contains("beta"));
    }

    #[test]
    fn cwd_guard_restores_previous_directory() {
        let _lock = TEST_CWD_LOCK.lock().unwrap();
        let previous = std::env::current_dir().unwrap();
        let tmp = TempDir::new("cwd-guard");

        {
            let _guard = CwdGuard::change_to(tmp.path());
            assert_eq!(std::env::current_dir().unwrap(), tmp.path());
        }

        assert_eq!(std::env::current_dir().unwrap(), previous);
    }

    #[test]
    fn temp_dir_drop_removes_created_directory() {
        let path = {
            let tmp = TempDir::new("temp-dir-drop");
            let path = tmp.path().to_path_buf();
            assert!(path.is_dir());
            path
        };

        assert!(!path.exists());
    }

    #[test]
    fn scrub_host_env_removes_known_terminal_keys() {
        let _lock = splashboard::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let keys = [
            "WT_SESSION",
            "GHOSTTY_RESOURCES_DIR",
            "KITTY_WINDOW_ID",
            "ALACRITTY_WINDOW_ID",
            "ALACRITTY_LOG",
            "WEZTERM_PANE",
            "TERM_PROGRAM",
        ];
        let _guard = EnvGuard::capture(&keys);

        keys.iter().for_each(|key| {
            // SAFETY: test-only env mutation serialized by TEST_ENV_LOCK.
            unsafe { std::env::set_var(key, "present") };
        });

        scrub_host_env();

        keys.iter()
            .for_each(|key| assert_eq!(std::env::var_os(key), None));
    }

    #[test]
    fn env_guard_restores_existing_values() {
        let _lock = splashboard::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // SAFETY: test-only env mutation serialized by TEST_ENV_LOCK.
        unsafe { std::env::set_var("WT_SESSION", "before") };

        {
            let _guard = EnvGuard::capture(&["WT_SESSION"]);
            // SAFETY: test-only env mutation serialized by TEST_ENV_LOCK.
            unsafe { std::env::set_var("WT_SESSION", "after") };
        }

        assert_eq!(
            std::env::var_os("WT_SESSION"),
            Some(OsString::from("before"))
        );
        // SAFETY: test-only env mutation serialized by TEST_ENV_LOCK.
        unsafe { std::env::remove_var("WT_SESSION") };
    }

    #[test]
    fn render_dashboards_surfaces_snapshot_render_errors_from_wrong_cwd() {
        let _lock = TEST_CWD_LOCK.lock().unwrap();
        let out = TempDir::new("render-error");
        let wrong_cwd = TempDir::new("wrong-cwd");
        let _cwd = CwdGuard::change_to(wrong_cwd.path());

        let err = render_dashboards(out.path()).unwrap_err();
        let message = format!("{err:#}");

        assert!(message.contains("src/templates/home_splash.toml"));
        assert!(!out.path().join("home_splash.html").exists());
    }

    #[test]
    fn render_dashboards_surfaces_file_create_errors() {
        let _lock = TEST_CWD_LOCK.lock().unwrap();
        let out = TempDir::new("render-create-error");
        let blocked = out.path().join("home_splash.html");
        fs::create_dir_all(&blocked).unwrap();
        let _cwd = CwdGuard::change_to(&workspace_root());

        let err = render_dashboards(out.path()).unwrap_err();
        let message = format!("{err:#}");

        assert!(message.contains("create"));
        assert!(message.contains(&blocked.display().to_string()));
    }
}
