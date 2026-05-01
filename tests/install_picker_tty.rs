#![cfg(target_os = "linux")]

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use splashboard::templates;
use tempfile::TempDir;

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[link(name = "util")]
unsafe extern "C" {
    fn openpty(
        amaster: *mut i32,
        aslave: *mut i32,
        name: *mut core::ffi::c_char,
        termp: *const core::ffi::c_void,
        winp: *const Winsize,
    ) -> i32;
}

struct TestPty {
    master: File,
    slave: File,
}

impl TestPty {
    fn new() -> io::Result<Self> {
        let mut master = -1;
        let mut slave = -1;
        let size = Winsize {
            ws_row: 40,
            ws_col: 120,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        cvt(unsafe {
            openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        })?;

        Ok(Self {
            master: unsafe { File::from_raw_fd(master) },
            slave: unsafe { File::from_raw_fd(slave) },
        })
    }

    fn spawn_cli(&self, cwd: &Path, home: &Path, args: &[&str]) -> io::Result<Child> {
        let stdin = self.slave.try_clone()?;
        let stdout = self.slave.try_clone()?;
        let stderr = self.slave.try_clone()?;
        let home = home.display().to_string();

        Command::new(env!("CARGO_BIN_EXE_splashboard"))
            .current_dir(cwd)
            .args(args)
            .env("HOME", &home)
            .env("SPLASHBOARD_HOME", &home)
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
    }

    fn spawn_install(&self, home: &Path) -> io::Result<Child> {
        self.spawn_cli(
            workspace_root().as_path(),
            home,
            &["install", "--shell", "zsh"],
        )
    }

    fn send_input(&self, input: &'static [u8]) -> thread::JoinHandle<()> {
        let mut master = self.master.try_clone().unwrap();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            input.iter().for_each(|byte| {
                if master.write_all(&[*byte]).is_err() {
                    return;
                }
                let _ = master.flush();
                thread::sleep(Duration::from_millis(120));
            });
        })
    }

    fn drain_output(&self) -> thread::JoinHandle<Vec<u8>> {
        let mut master = self.master.try_clone().unwrap();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            loop {
                match master.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => bytes.extend_from_slice(&buf[..n]),
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(err) if err.raw_os_error() == Some(5) => break,
                    Err(_) => break,
                }
            }
            bytes
        })
    }
}

fn cvt(ret: i32) -> io::Result<i32> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "interactive install timed out",
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn run_install(home: &TempDir, input: &'static [u8]) -> io::Result<(ExitStatus, String)> {
    let pty = TestPty::new()?;
    let reader = pty.drain_output();
    let writer = pty.send_input(input);
    let mut child = pty.spawn_install(home.path())?;
    let status = wait_for_exit(&mut child, Duration::from_secs(15))?;
    writer.join().unwrap();
    drop(child);
    drop(pty);
    let output = String::from_utf8_lossy(&reader.join().unwrap()).into_owned();
    Ok((status, output))
}

fn run_cli(cwd: &Path, home: &TempDir, args: &[&str]) -> io::Result<(ExitStatus, String)> {
    let pty = TestPty::new()?;
    let reader = pty.drain_output();
    let mut child = pty.spawn_cli(cwd, home.path(), args)?;
    let status = wait_for_exit(&mut child, Duration::from_secs(15))?;
    drop(child);
    drop(pty);
    let output = String::from_utf8_lossy(&reader.join().unwrap()).into_owned();
    Ok((status, output))
}

#[test]
fn interactive_install_picker_covers_confirm_and_cancel_paths() {
    let success_dir = tempfile::tempdir().unwrap();
    let (success, success_output) = run_install(&success_dir, b"\rj\r\rj \ry").unwrap();

    assert!(
        success.success(),
        "interactive install failed:\n{success_output}"
    );

    let home = success_dir.path().join("home.dashboard.toml");
    let project = success_dir.path().join("project.dashboard.toml");
    let settings = success_dir.path().join("settings.toml");
    let rc = success_dir.path().join(".zshrc");

    assert_eq!(
        std::fs::read_to_string(&home).unwrap(),
        templates::find("home_splash").unwrap().body
    );
    assert_eq!(
        std::fs::read_to_string(&project).unwrap(),
        templates::find("project_github").unwrap().body
    );

    let settings_body = std::fs::read_to_string(&settings).unwrap();
    assert!(settings_body.contains("[general]"));
    assert!(settings_body.contains("wait_for_fresh = true"));
    assert!(!settings_body.contains("[theme]"));
    assert!(rc.is_file());

    let cancel_dir = tempfile::tempdir().unwrap();
    let (cancel, cancel_output) = run_install(&cancel_dir, b"\rj\r\rj \rn").unwrap();

    assert!(
        !cancel.success(),
        "cancelled install unexpectedly succeeded:\n{cancel_output}"
    );
    assert!(cancel_output.contains("install cancelled"));
    assert!(!cancel_dir.path().join("home.dashboard.toml").exists());
    assert!(!cancel_dir.path().join("project.dashboard.toml").exists());
    assert!(!cancel_dir.path().join("settings.toml").exists());
    assert!(!cancel_dir.path().join(".zshrc").exists());
}

#[test]
fn tty_on_cd_renders_local_dashboard_and_populates_cache() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(
        cwd.path().join(".splashboard.toml"),
        r#"
[[widget]]
id = "hello"
fetcher = "basic_static"
format = "Hello runtime"
render = "text_plain"

[[row]]
height = { length = 3 }
[[row.child]]
widget = "hello"
"#,
    )
    .unwrap();

    let (status, output) = run_cli(cwd.path(), &home, &["--on-cd"]).unwrap();
    assert!(status.success(), "render failed:\n{output}");
    assert!(!output.is_empty(), "expected TTY render output");

    let cache = home.path().join("cache");
    assert!(cache.is_dir());
}
