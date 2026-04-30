use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::ValueEnum;
use tokio::process::{Child, Command};

use crate::config::{Config, DashboardConfig, DashboardSource, SettingsConfig};
use crate::paths;
use crate::runtime;

/// Which dashboard the parent resolved. Passed to the daemon subprocess so it loads the same
/// source without having to re-resolve from scratch (avoids drift if CWD changed between
/// parent spawn and daemon start).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DashboardKind {
    Local,
    Home,
    Project,
}

impl DashboardSource {
    pub fn kind(&self) -> DashboardKind {
        match self {
            Self::Local(_) => DashboardKind::Local,
            Self::Home => DashboardKind::Home,
            Self::Project => DashboardKind::Project,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Local(p) => Some(p),
            _ => None,
        }
    }
}

/// Entry point for the `fetch-only` subcommand. Run by the detached child process: resolves the
/// dashboard, drives the fetchers, writes the cache, and exits. The parent splashboard invocation
/// either detaches from this child and exits, or (in `--wait` mode) blocks on it with a deadline.
pub async fn run_fetch_only(kind: DashboardKind, path: Option<&Path>) -> io::Result<()> {
    let (dashboard, ident) = load_dashboard(kind, path)?;
    let settings = load_settings_or_default();
    let config = Config::from_parts(settings, dashboard);
    let ident_ref = ident.as_ref().map(|(p, h)| (p.as_path(), h.as_str()));
    runtime::fetch_and_persist(&config, ident_ref).await;
    Ok(())
}

pub fn spawn_fetch_daemon(source: &DashboardSource) -> io::Result<Child> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("fetch-only")
        .arg("--kind")
        .arg(match source.kind() {
            DashboardKind::Local => "local",
            DashboardKind::Home => "home",
            DashboardKind::Project => "project",
        });
    if let Some(p) = source.path() {
        cmd.arg("--path").arg(p);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    detach(cmd.as_std_mut());
    cmd.spawn()
}

fn load_dashboard(
    kind: DashboardKind,
    path: Option<&Path>,
) -> io::Result<(DashboardConfig, Option<(PathBuf, String)>)> {
    match kind {
        DashboardKind::Local => {
            let p =
                path.ok_or_else(|| io::Error::other("fetch-only --kind local requires --path"))?;
            let (d, h) = crate::trust::load_dashboard_and_hash(p)?;
            Ok((d, Some((p.to_path_buf(), h))))
        }
        DashboardKind::Home => Ok((load_home_dashboard_or_baked(), None)),
        DashboardKind::Project => Ok((load_project_dashboard_or_baked(), None)),
    }
}

fn load_home_dashboard_or_baked() -> DashboardConfig {
    paths::home_dashboard_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| DashboardConfig::parse(&s).ok())
        .unwrap_or_else(DashboardConfig::default_home)
}

fn load_project_dashboard_or_baked() -> DashboardConfig {
    paths::project_dashboard_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| DashboardConfig::parse(&s).ok())
        .unwrap_or_else(DashboardConfig::default_project)
}

fn load_settings_or_default() -> SettingsConfig {
    paths::settings_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| SettingsConfig::parse(&s).ok())
        .unwrap_or_else(SettingsConfig::default_baked)
}

#[cfg(unix)]
fn detach(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // New process group so SIGINT to the shell doesn't tear the daemon down mid-fetch.
    cmd.process_group(0);
}

#[cfg(windows)]
fn detach(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn detach(_cmd: &mut std::process::Command) {}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::paths::TEST_ENV_LOCK;
    use tempfile::tempdir;

    struct SplashboardHomeGuard(Option<OsString>);

    impl SplashboardHomeGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os("SPLASHBOARD_HOME");
            unsafe {
                std::env::set_var("SPLASHBOARD_HOME", path);
            }
            Self(previous)
        }
    }

    impl Drop for SplashboardHomeGuard {
        fn drop(&mut self) {
            unsafe {
                match self.0.take() {
                    Some(value) => std::env::set_var("SPLASHBOARD_HOME", value),
                    None => std::env::remove_var("SPLASHBOARD_HOME"),
                }
            }
        }
    }

    fn minimal_dashboard(id: &str) -> String {
        format!(
            r#"
[[widget]]
id = "{id}"
fetcher = "basic_static"
format = "hello {id}"
render = "text_plain"

[[row]]
[[row.child]]
widget = "{id}"
"#
        )
    }

    #[test]
    fn dashboard_source_helpers_preserve_kind_and_local_path() {
        let local = DashboardSource::Local(PathBuf::from("local.toml"));
        assert_eq!(local.kind(), DashboardKind::Local);
        assert_eq!(local.path(), Some(Path::new("local.toml")));
        assert_eq!(DashboardSource::Home.kind(), DashboardKind::Home);
        assert_eq!(DashboardSource::Home.path(), None);
        assert_eq!(DashboardSource::Project.kind(), DashboardKind::Project);
        assert_eq!(DashboardSource::Project.path(), None);
    }

    #[test]
    fn load_dashboard_local_returns_hash_for_valid_file() {
        let dir = tempdir().unwrap();
        let local = dir.path().join("local.dashboard.toml");
        std::fs::write(&local, minimal_dashboard("local")).unwrap();

        let (dashboard, ident) = load_dashboard(DashboardKind::Local, Some(&local)).unwrap();
        assert_eq!(dashboard.widgets[0].id, "local");
        let (path, hash) = ident.unwrap();
        assert_eq!(path, local);
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn home_project_and_settings_loaders_use_files_then_fallbacks() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let _guard = SplashboardHomeGuard::set(dir.path());
        let default_home = DashboardConfig::default_home();
        let default_project = DashboardConfig::default_project();
        let default_settings = SettingsConfig::default_baked();

        let (home_missing, _) = load_dashboard(DashboardKind::Home, None).unwrap();
        assert_eq!(home_missing.widgets.len(), default_home.widgets.len());
        assert_eq!(home_missing.rows.len(), default_home.rows.len());
        let (project_missing, _) = load_dashboard(DashboardKind::Project, None).unwrap();
        assert_eq!(project_missing.widgets.len(), default_project.widgets.len());
        assert_eq!(project_missing.rows.len(), default_project.rows.len());
        assert_eq!(
            load_settings_or_default().general.auto_home,
            default_settings.general.auto_home
        );

        std::fs::write(
            paths::home_dashboard_path().unwrap(),
            minimal_dashboard("home"),
        )
        .unwrap();
        std::fs::write(
            paths::project_dashboard_path().unwrap(),
            minimal_dashboard("project"),
        )
        .unwrap();
        std::fs::write(
            paths::settings_path().unwrap(),
            "[general]\nauto_home = false\nauto_on_cd = false\n",
        )
        .unwrap();

        let (home_loaded, ident) = load_dashboard(DashboardKind::Home, None).unwrap();
        assert_eq!(home_loaded.widgets[0].id, "home");
        assert!(ident.is_none());
        let (project_loaded, ident) = load_dashboard(DashboardKind::Project, None).unwrap();
        assert_eq!(project_loaded.widgets[0].id, "project");
        assert!(ident.is_none());
        let loaded_settings = load_settings_or_default();
        assert!(!loaded_settings.general.auto_home);
        assert!(!loaded_settings.general.auto_on_cd);

        std::fs::write(paths::home_dashboard_path().unwrap(), "not valid toml").unwrap();
        std::fs::write(paths::project_dashboard_path().unwrap(), "not valid toml").unwrap();
        std::fs::write(paths::settings_path().unwrap(), "not valid toml").unwrap();

        let (home_invalid, _) = load_dashboard(DashboardKind::Home, None).unwrap();
        assert_eq!(home_invalid.widgets.len(), default_home.widgets.len());
        assert_eq!(home_invalid.rows.len(), default_home.rows.len());
        let (project_invalid, _) = load_dashboard(DashboardKind::Project, None).unwrap();
        assert_eq!(project_invalid.widgets.len(), default_project.widgets.len());
        assert_eq!(project_invalid.rows.len(), default_project.rows.len());
        let invalid_settings = load_settings_or_default();
        assert_eq!(
            invalid_settings.general.auto_home,
            default_settings.general.auto_home
        );
        assert_eq!(
            invalid_settings.general.auto_on_cd,
            default_settings.general.auto_on_cd
        );
    }

    #[tokio::test]
    async fn run_fetch_only_home_uses_baked_default() {
        // No file under SPLASHBOARD_HOME: falls back to the empty baked home dashboard. Runs
        // to completion rather than erroring — the daemon has no stdio to report over. The
        // temp-dir rebase doesn't mutate `SPLASHBOARD_HOME` (that would need the env lock and
        // would drag us into holding-mutex-across-await warnings); we rely on the empty-HOME
        // branch producing the baked default either way.
        run_fetch_only(DashboardKind::Home, None).await.unwrap();
    }

    #[tokio::test]
    async fn run_fetch_only_local_bubbles_parse_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "this is [not valid toml").unwrap();
        let err = run_fetch_only(DashboardKind::Local, Some(&bad))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[tokio::test]
    async fn run_fetch_only_local_without_path_errors() {
        let err = run_fetch_only(DashboardKind::Local, None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test]
    fn run_fetch_only_local_valid_dashboard_persists_cache() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let _guard = SplashboardHomeGuard::set(dir.path());
        let local = dir.path().join("local.dashboard.toml");
        std::fs::write(&local, minimal_dashboard("local")).unwrap();

        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_fetch_only(DashboardKind::Local, Some(&local)))
            .unwrap();

        let cache_dir = paths::cache_dir().unwrap();
        assert!(cache_dir.exists());
        assert!(std::fs::read_dir(cache_dir).unwrap().next().is_some());
    }

    #[tokio::test]
    async fn spawn_fetch_daemon_spawns_children_for_each_dashboard_kind() {
        let dir = tempdir().unwrap();
        let local = dir.path().join("local.dashboard.toml");
        std::fs::write(&local, minimal_dashboard("local")).unwrap();
        let sources = vec![
            DashboardSource::Home,
            DashboardSource::Project,
            DashboardSource::Local(local),
        ];

        for source in sources {
            let mut child = spawn_fetch_daemon(&source).unwrap();
            let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await
                .unwrap()
                .unwrap();
            assert!(!status.success());
        }
    }

    #[test]
    fn detach_chains_onto_command_builder() {
        // We only verify that `detach` composes with the rest of the Command builder without
        // panicking. Actually spawning a probe binary is left to manual / integration testing
        // because the runner's $PATH and filesystem sandboxing differ across CI hosts.
        let mut cmd = std::process::Command::new("splashboard-nonexistent-probe");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut cmd);
        // Further builder calls must still work after detach.
        cmd.arg("--probe");
    }
}
