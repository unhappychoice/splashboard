//! Idempotent shell-rc wiring. Writes a marker-delimited block that delegates to
//! `splashboard init <shell>`; re-running `install` detects the block and replaces
//! it in place rather than stacking duplicates.

use std::io;
use std::path::{Path, PathBuf};

use crate::shell::{self, Shell};

pub(crate) const MARKER_OPEN: &str = "# >>> splashboard >>>";
pub(crate) const MARKER_CLOSE: &str = "# <<< splashboard <<<";

pub(crate) struct RcReport {
    action: RcAction,
    rc_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RcAction {
    /// Existing rc already had an equivalent marker block — no file change needed.
    UpToDate,
    /// Appended a new marker block to an existing rc.
    Appended,
    /// Replaced the contents of an existing marker block.
    Replaced,
    /// Created the rc file (path did not previously exist).
    Created,
    /// Couldn't locate the rc path at all (no `$HOME`, no `--rc-path` override).
    UnknownPath,
}

impl RcReport {
    pub(crate) fn describe(&self) {
        let Some(path) = &self.rc_path else {
            println!("Shell rc:  (path unknown — skipped)");
            return;
        };
        let display = path.display();
        match self.action {
            RcAction::UpToDate => println!("Shell rc:  {display} (already wired)"),
            RcAction::Appended => println!("Shell rc:  {display} (wired — appended block)"),
            RcAction::Replaced => println!("Shell rc:  {display} (wired — refreshed block)"),
            RcAction::Created => println!("Shell rc:  {display} (created)"),
            RcAction::UnknownPath => println!("Shell rc:  {display} (skipped)"),
        }
    }

    pub(crate) fn rc_path_display(&self) -> String {
        match &self.rc_path {
            Some(p) => p.display().to_string(),
            None => "(rc path unknown)".to_string(),
        }
    }
}

/// Wires the rc file, creating it if missing. Appends a marker-delimited block sourcing
/// `splashboard init <shell>`; re-runs replace the existing block in place. There's no
/// "ask before creating" step — the user ran `splashboard install`, so wiring the rc is
/// the job; if they didn't want that they'd have run `splashboard init` and done it by
/// hand. Existing rc contents outside the marker block are always preserved.
pub(crate) fn wire_shell_rc(shell: Shell, override_path: Option<PathBuf>) -> io::Result<RcReport> {
    let Some(rc_path) = override_path.or_else(|| shell::default_rc_path(shell)) else {
        return Ok(RcReport {
            action: RcAction::UnknownPath,
            rc_path: None,
        });
    };
    let block = format_block(shell);
    let existing = match std::fs::read_to_string(&rc_path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let action = apply_block(&rc_path, existing.as_deref(), &block)?;
    Ok(RcReport {
        action,
        rc_path: Some(rc_path),
    })
}

fn apply_block(rc_path: &Path, existing: Option<&str>, block: &str) -> io::Result<RcAction> {
    match existing {
        None => {
            if let Some(parent) = rc_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(rc_path, block)?;
            Ok(RcAction::Created)
        }
        Some(contents) => match replace_or_append(contents, block) {
            Outcome::UpToDate => Ok(RcAction::UpToDate),
            Outcome::Replaced(new) => {
                std::fs::write(rc_path, new)?;
                Ok(RcAction::Replaced)
            }
            Outcome::Appended(new) => {
                std::fs::write(rc_path, new)?;
                Ok(RcAction::Appended)
            }
        },
    }
}

enum Outcome {
    UpToDate,
    Replaced(String),
    Appended(String),
}

fn replace_or_append(contents: &str, block: &str) -> Outcome {
    match find_block(contents) {
        Some((start, end)) => {
            let existing_block = &contents[start..end];
            if existing_block.trim_end() == block.trim_end() {
                return Outcome::UpToDate;
            }
            let mut new = String::with_capacity(contents.len() + block.len());
            new.push_str(&contents[..start]);
            new.push_str(block);
            new.push_str(&contents[end..]);
            Outcome::Replaced(new)
        }
        None => {
            let mut new = String::with_capacity(contents.len() + block.len() + 1);
            new.push_str(contents);
            if !contents.ends_with('\n') && !contents.is_empty() {
                new.push('\n');
            }
            if !contents.is_empty() {
                new.push('\n');
            }
            new.push_str(block);
            Outcome::Appended(new)
        }
    }
}

/// Returns the byte range `[start, end)` covering the full marker block including its
/// trailing newline (if any), or `None` when the block isn't present. The range is
/// inclusive of the open marker line and exclusive of the line after the close marker,
/// so splicing `contents[..start] + new_block + contents[end..]` keeps surrounding
/// whitespace intact.
fn find_block(contents: &str) -> Option<(usize, usize)> {
    let start = contents.find(MARKER_OPEN)?;
    let after_open = start + MARKER_OPEN.len();
    let close_rel = contents[after_open..].find(MARKER_CLOSE)?;
    let close_abs = after_open + close_rel;
    // Consume up to and including the newline that terminates the close marker line.
    let end = contents[close_abs..]
        .find('\n')
        .map(|nl| close_abs + nl + 1)
        .unwrap_or(contents.len());
    Some((start, end))
}

fn format_block(shell: Shell) -> String {
    format!(
        "{open}\n# Added by `splashboard install`. Safe to remove.\n{line}\n{close}\n",
        open = MARKER_OPEN,
        line = shell::source_line(shell),
        close = MARKER_CLOSE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn appends_block_to_existing_rc() {
        let contents = "# some prior config\nalias foo=bar\n";
        let block = format_block(Shell::Zsh);
        let out = match replace_or_append(contents, &block) {
            Outcome::Appended(s) => s,
            _ => panic!("expected Appended"),
        };
        assert!(out.starts_with("# some prior config"));
        assert!(out.contains(MARKER_OPEN));
        assert!(out.contains(MARKER_CLOSE));
        assert!(out.contains("splashboard init zsh"));
    }

    #[test]
    fn replaces_existing_block_in_place() {
        let stale = format!(
            "\
prior\n\
{MARKER_OPEN}\n\
eval \"$(splashboard init bash)\"\n\
{MARKER_CLOSE}\n\
trailing\n"
        );
        let block = format_block(Shell::Zsh);
        let out = match replace_or_append(&stale, &block) {
            Outcome::Replaced(s) => s,
            other => panic!(
                "expected Replaced, got {other:?}",
                other = discriminant(&other)
            ),
        };
        // Surrounding content is preserved and the new shell line is in.
        assert!(out.starts_with("prior\n"));
        assert!(out.trim_end().ends_with("trailing"));
        assert!(out.contains("splashboard init zsh"));
        assert!(!out.contains("splashboard init bash"));
        // Block appears exactly once.
        assert_eq!(out.matches(MARKER_OPEN).count(), 1);
    }

    #[test]
    fn no_change_when_block_already_matches() {
        let block = format_block(Shell::Fish);
        let contents = format!("prior\n{block}\ntrailing\n");
        match replace_or_append(&contents, &block) {
            Outcome::UpToDate => {}
            _ => panic!("expected UpToDate when block already matches"),
        }
    }

    #[test]
    fn creates_rc_when_missing() {
        let dir = tempdir().unwrap();
        let rc = dir.path().join(".zshrc");
        let report = wire_shell_rc(Shell::Zsh, Some(rc.clone())).unwrap();
        assert_eq!(report.action, RcAction::Created);
        assert!(rc.exists());
        assert!(
            std::fs::read_to_string(&rc)
                .unwrap()
                .contains("splashboard init zsh")
        );
    }

    #[test]
    fn creates_parent_directory_if_missing() {
        let dir = tempdir().unwrap();
        // Nested fish-style path — parent `config/fish/` does not exist yet.
        let rc = dir.path().join("config").join("fish").join("config.fish");
        let report = wire_shell_rc(Shell::Fish, Some(rc.clone())).unwrap();
        assert_eq!(report.action, RcAction::Created);
        assert!(rc.exists());
    }

    fn discriminant(o: &Outcome) -> &'static str {
        match o {
            Outcome::UpToDate => "UpToDate",
            Outcome::Replaced(_) => "Replaced",
            Outcome::Appended(_) => "Appended",
        }
    }
}
