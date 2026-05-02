use std::path::{Path, PathBuf};
use std::process::Command;
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

    fn file(label: &str) -> io::Result<Self> {
        let path = unique_path(label);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, [])?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = match fs::metadata(&self.path) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(&self.path),
            Ok(_) => fs::remove_file(&self.path),
            Err(_) => Ok(()),
        };
    }
}

fn unique_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splashboard-xtask-{label}-{unique}-{}",
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
fn xtask_cli_generates_reference_and_snapshot_outputs() {
    let out = TempPath::dir("docs").unwrap();
    let rendered = TempPath::dir("rendered").unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(workspace_root())
        .arg("--out")
        .arg(out.path())
        .arg("--rendered-out")
        .arg(rendered.path())
        .status()
        .unwrap();

    assert!(status.success());
    assert!(out.path().join("matrix.md").is_file());
    assert!(out.path().join("fetchers/clock/clock.md").is_file());
    assert!(out.path().join("renderers/text/text_plain.md").is_file());
    assert!(rendered.path().join("home_splash.html").is_file());
    assert!(rendered.path().join("project_github.html").is_file());

    let html = fs::read_to_string(rendered.path().join("home_splash.html")).unwrap();
    assert!(html.contains("<pre class=\"splash-snapshot\">"));
}

#[test]
fn xtask_cli_surfaces_render_output_creation_errors() {
    let out = TempPath::dir("docs-error").unwrap();
    let rendered = TempPath::file("rendered-error").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .current_dir(workspace_root())
        .arg("--out")
        .arg(out.path())
        .arg("--rendered-out")
        .arg(rendered.path())
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("create"));
    assert!(stderr.contains(&rendered.path().display().to_string()));
}
