use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

struct TempPath {
    path: PathBuf,
}

impl TempPath {
    fn dir(label: &str) -> io::Result<Self> {
        let path = unique_path(label);
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splashboard-main-{label}-{unique}-{}",
        std::process::id()
    ))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(workspace_root())
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn init_subcommand_prints_zsh_hook_snippet() {
    let output = run_cli(&["init", "zsh"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("add-zsh-hook chpwd"));
    assert!(stdout.contains("splashboard --on-cd"));
}

#[test]
fn license_subcommand_prints_own_license_text() {
    let output = run_cli(&["license", "--own"]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ISC License"));
    assert!(stdout.contains("Permission to use, copy, modify"));
}

#[test]
fn catalog_subcommand_covers_detail_and_unknown_renderer_paths() {
    let detail = run_cli(&["catalog", "fetcher", "clock"]);
    assert!(detail.status.success());

    let detail_stdout = String::from_utf8_lossy(&detail.stdout);
    assert!(detail_stdout.contains("Fetcher `clock`"));
    assert!(detail_stdout.contains("Compatible renderers"));

    let missing = run_cli(&["catalog", "renderer", "nope"]);
    assert_eq!(missing.status.code(), Some(2));

    let missing_stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(missing_stderr.contains("unknown renderer: nope"));
    assert!(missing_stderr.contains("available:"));
}

#[test]
fn trust_and_revoke_report_when_no_local_dashboard_exists() {
    let cwd = TempPath::dir("no-dashboard").unwrap();

    let trust = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(cwd.path())
        .arg("trust")
        .output()
        .unwrap();
    assert!(trust.status.success());
    assert!(String::from_utf8_lossy(&trust.stderr).contains("no project-local dashboard found"));

    let revoke = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(cwd.path())
        .arg("revoke")
        .output()
        .unwrap();
    assert!(revoke.status.success());
    assert!(String::from_utf8_lossy(&revoke.stderr).contains("no project-local dashboard found"));
}

#[test]
fn list_trusted_and_fetch_only_succeed_with_isolated_home() {
    let home = TempPath::dir("home").unwrap();
    let home_path = home.path().display().to_string();

    let list = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(workspace_root())
        .arg("list-trusted")
        .env("SPLASHBOARD_HOME", &home_path)
        .output()
        .unwrap();
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).is_empty());

    let fetch_only = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(workspace_root())
        .args(["fetch-only", "--kind", "home"])
        .env("SPLASHBOARD_HOME", &home_path)
        .output()
        .unwrap();
    assert!(fetch_only.status.success());
    assert!(String::from_utf8_lossy(&fetch_only.stdout).is_empty());
}

#[test]
fn bare_invocation_exits_cleanly_without_a_tty() {
    let output = run_cli(&[]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}
