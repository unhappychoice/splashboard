#![cfg(test)]
//! Test-only repo builder shared by every git fetcher's tests. Uses the `git` CLI via
//! `std::process::Command` rather than gix write APIs so we don't need extra gix features just
//! for fixtures. CI runs with git installed (everywhere we care about).

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

pub fn make_repo() -> (TempDir, gix::Repository) {
    let tmp = tempfile::tempdir().expect("tempdir");
    run(tmp.path(), &["init", "-q", "-b", "main"]);
    run(tmp.path(), &["config", "user.email", "test@example.com"]);
    run(tmp.path(), &["config", "user.name", "Test"]);
    run(tmp.path(), &["config", "commit.gpgsign", "false"]);
    run(tmp.path(), &["config", "tag.gpgsign", "false"]);
    let repo = gix::discover(tmp.path()).expect("discover");
    (tmp, repo)
}

pub fn commit(repo: &gix::Repository, msg: &str) {
    let path = repo.workdir().expect("workdir");
    let file = path.join("README.md");
    let prev = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(&file, format!("{prev}{msg}\n")).unwrap();
    run(path, &["add", "."]);
    run(path, &["commit", "-q", "-m", msg]);
}

pub fn commit_as(repo: &gix::Repository, msg: &str, author: &str, email: &str) {
    let path = repo.workdir().expect("workdir");
    let file = path.join("README.md");
    let prev = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(&file, format!("{prev}{msg}\n")).unwrap();
    run(path, &["add", "."]);
    run(
        path,
        &[
            "-c",
            &format!("user.name={author}"),
            "-c",
            &format!("user.email={email}"),
            "commit",
            "-q",
            "-m",
            msg,
        ],
    );
}

pub fn commit_touching(repo: &gix::Repository, file_rel: &str, msg: &str) {
    let path = repo.workdir().expect("workdir");
    let file = path.join(file_rel);
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let prev = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(&file, format!("{prev}{msg}\n")).unwrap();
    run(path, &["add", "."]);
    run(path, &["commit", "-q", "-m", msg]);
}

pub fn tag(repo: &gix::Repository, name: &str) {
    run(repo.workdir().unwrap(), &["tag", name]);
}

pub fn stash(repo: &gix::Repository) {
    let path = repo.workdir().unwrap();
    let file = path.join("README.md");
    let prev = std::fs::read_to_string(&file).unwrap_or_default();
    std::fs::write(&file, format!("{prev}wip\n")).unwrap();
    run(
        path,
        &["stash", "push", "--include-untracked", "-m", "wip"],
    );
}

pub fn dirty_write(repo: &gix::Repository, file_rel: &str, contents: &str) {
    let path = repo.workdir().unwrap();
    std::fs::write(path.join(file_rel), contents).unwrap();
}

fn run(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git should be on PATH for tests");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
