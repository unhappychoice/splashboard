use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
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

fn run_cli_with_input(cwd: &Path, args: &[&str], envs: &[(&str, &str)], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(cwd)
        .args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[cfg(target_os = "linux")]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "linux")]
fn run_cli_tty(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let env_prefix = std::iter::once(("TERM", "xterm-256color"))
        .chain(envs.iter().copied())
        .map(|(key, value)| format!("{key}={}", shell_quote(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let binary = shell_quote(env!("CARGO_BIN_EXE_splashboard"));
    let args = args
        .iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let command = if env_prefix.is_empty() {
        format!("{binary} {args}")
    } else {
        format!("env {env_prefix} {binary} {args}")
    };

    Command::new("script")
        .current_dir(cwd)
        .args(["-qefc", &command, "/dev/null"])
        .output()
        .unwrap()
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

// `dirs::home_dir()` on Windows resolves via the Known Folder API, which ignores the
// `HOME` env override the test relies on, so the rc path lands outside the temp HOME.
#[cfg(not(target_os = "windows"))]
#[test]
fn install_subcommand_writes_dashboards_settings_and_rc_in_non_tty_mode() {
    let home = TempPath::dir("install-home").unwrap();
    let home_path = home.path().display().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(workspace_root())
        .args([
            "install",
            "--shell",
            "zsh",
            "--home-template",
            "home_splash",
            "--project-template",
            "project_github",
            "--theme",
            "tokyo_night",
            "--no-bg",
            "--wait",
        ])
        .env("HOME", &home_path)
        .env("SPLASHBOARD_HOME", &home_path)
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("splashboard install — done."));
    assert!(stdout.contains("theme = tokyo_night"));
    assert!(home.path().join("home.dashboard.toml").is_file());
    assert!(home.path().join("project.dashboard.toml").is_file());
    assert!(home.path().join("settings.toml").is_file());
    assert!(home.path().join(".zshrc").is_file());
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

#[cfg(target_os = "linux")]
#[test]
fn tty_invocation_respects_auto_home_false() {
    let home = TempPath::dir("tty-home-auto-home").unwrap();
    let home_path = home.path().display().to_string();
    fs::write(
        home.path().join("settings.toml"),
        "[general]\nauto_home = false\n",
    )
    .unwrap();

    let output = run_cli_tty(
        workspace_root().as_path(),
        &[],
        &[("HOME", &home_path), ("SPLASHBOARD_HOME", &home_path)],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn tty_on_cd_invocation_exits_cleanly_without_a_dashboard_source() {
    let home = TempPath::dir("tty-home-on-cd-none").unwrap();
    let cwd = TempPath::dir("tty-cwd-on-cd-none").unwrap();
    let home_path = home.path().display().to_string();

    let output = run_cli_tty(
        cwd.path(),
        &["--on-cd"],
        &[("HOME", &home_path), ("SPLASHBOARD_HOME", &home_path)],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn tty_on_cd_invocation_respects_auto_on_cd_false() {
    let home = TempPath::dir("tty-home-auto-on-cd").unwrap();
    let home_path = home.path().display().to_string();
    fs::write(
        home.path().join("settings.toml"),
        "[general]\nauto_on_cd = false\n",
    )
    .unwrap();

    let output = run_cli_tty(
        workspace_root().as_path(),
        &["--on-cd"],
        &[("HOME", &home_path), ("SPLASHBOARD_HOME", &home_path)],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn trust_subcommand_declines_when_user_answers_no() {
    let home = TempPath::dir("trust-home-no").unwrap();
    let cwd = TempPath::dir("trust-cwd-no").unwrap();
    let dashboard = cwd.path().join(".splashboard.toml");
    fs::write(&dashboard, minimal_dashboard()).unwrap();
    let home_path = home.path().display().to_string();

    let output = run_cli_with_input(
        cwd.path(),
        &["trust"],
        &[("SPLASHBOARD_HOME", &home_path)],
        "n\n",
    );
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Trust this dashboard? [y/N]"));
    assert!(stdout.contains("not trusted"));
    assert!(!home.path().join("trust.toml").exists());
}

// On Windows `list-trusted` prints a canonicalized `\\?\` UNC path while
// `dashboard.display()` doesn't, so the substring check fails despite the entry being
// persisted correctly.
#[cfg(not(target_os = "windows"))]
#[test]
fn trust_subcommand_persists_entry_when_user_answers_yes() {
    let home = TempPath::dir("trust-home-yes").unwrap();
    let cwd = TempPath::dir("trust-cwd-yes").unwrap();
    let dashboard = cwd.path().join(".splashboard.toml");
    fs::write(&dashboard, minimal_dashboard()).unwrap();
    let home_path = home.path().display().to_string();

    let trust = run_cli_with_input(
        cwd.path(),
        &["trust"],
        &[("SPLASHBOARD_HOME", &home_path)],
        "yes\n",
    );
    assert!(trust.status.success());

    let trust_stdout = String::from_utf8_lossy(&trust.stdout);
    assert!(trust_stdout.contains("Trust this dashboard? [y/N]"));
    assert!(trust_stdout.contains("trusted:"));
    assert!(home.path().join("trust.toml").is_file());

    let list = Command::new(env!("CARGO_BIN_EXE_splashboard"))
        .current_dir(cwd.path())
        .arg("list-trusted")
        .env("SPLASHBOARD_HOME", &home_path)
        .output()
        .unwrap();
    assert!(list.status.success());
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains(&dashboard.display().to_string()));
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
