use std::io;
use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command};

use crate::config::Config;
use crate::runtime;

/// Entry point for the `fetch-only` subcommand. Run by the detached child process: resolves the
/// config, drives the fetchers, writes the cache, and exits. The parent splashboard invocation
/// either detaches from this child and exits, or (in `--wait` mode) blocks on it with a deadline.
pub async fn run_fetch_only(config_path: Option<&Path>) -> io::Result<()> {
    let config = match config_path {
        Some(p) => Config::load_or_default(p).map_err(io::Error::other)?,
        None => Config::default_baked(),
    };
    runtime::fetch_and_persist(&config, config_path).await;
    Ok(())
}

pub fn spawn_fetch_daemon(config_path: Option<&Path>) -> io::Result<Child> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("fetch-only");
    if let Some(p) = config_path {
        cmd.arg("--config").arg(p);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    detach(cmd.as_std_mut());
    cmd.spawn()
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
    use super::*;

    #[tokio::test]
    async fn run_fetch_only_with_missing_path_uses_baked_default() {
        // Passing None must fall back to the built-in config rather than erroring.
        run_fetch_only(None).await.unwrap();
    }

    #[tokio::test]
    async fn run_fetch_only_bubbles_parse_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "this is [not valid toml").unwrap();
        let err = run_fetch_only(Some(&bad)).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
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
