//! Shared index walker for the `code_*` family. Iterates tracked files in the repo discovered
//! from CWD, skipping committed-vendored trees, generated outputs, machine-written lock files,
//! oversized files, and binaries. Each surviving file's bytes are handed to the visitor.
//!
//! Untracked / `.gitignore`-d files are filtered for free by walking the index. The
//! [`EXCLUDED_DIRS`] and [`EXCLUDED_FILENAMES`] lists are the second line of defense for
//! noise that's been committed anyway and would otherwise inflate metrics
//! (`Cargo.lock` alone is ~6k lines on a typical Rust repo).

use super::super::FetchError;
use super::super::git::fail;

const FILE_SIZE_CAP: u64 = 1024 * 1024;
const MAX_FILES_SCANNED: usize = 5_000;

pub const EXCLUDED_DIRS: &[&str] = &[
    // Vendored / dependency trees
    "node_modules",
    "vendor",
    "third_party",
    "Pods",
    // Build / generated output
    "target",
    "dist",
    "build",
    "out",
    "_build",
    "_site",
    "DerivedData",
    // Framework caches that occasionally get committed
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".cache",
    // Coverage reports
    "coverage",
    "htmlcov",
    // Test artefacts
    "__snapshots__",
    "__pycache__",
    // Repo metadata
    ".git",
];

/// Filenames recognised as machine-written / non-human-authored noise. Match is on the basename
/// (`Cargo.lock`, `package-lock.json`) so the file is skipped wherever it lives in the tree.
/// Lockfiles inflate LOC counts dramatically (`Cargo.lock` is ~6k lines on this repo) and never
/// carry meaningful TODOs, so excluding them benefits every `code_*` fetcher.
pub const EXCLUDED_FILENAMES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "Gemfile.lock",
    "composer.lock",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "go.sum",
    "flake.lock",
    "mix.lock",
    "Podfile.lock",
];

/// Visit each tracked, regular, non-binary, in-bounds file in the repo's index. Stops after
/// [`MAX_FILES_SCANNED`] files; skips files larger than [`FILE_SIZE_CAP`]; skips paths whose
/// any segment matches [`EXCLUDED_DIRS`].
pub fn for_each_tracked_file<F>(repo: &gix::Repository, mut visit: F) -> Result<(), FetchError>
where
    F: FnMut(&str, &[u8]),
{
    let Some(workdir) = repo.workdir().map(|p| p.to_path_buf()) else {
        return Ok(());
    };
    let index = repo.index_or_load_from_head_or_empty().map_err(fail)?;
    let mut scanned = 0usize;
    for entry in index.entries() {
        if scanned >= MAX_FILES_SCANNED {
            break;
        }
        if !is_regular_file(entry.mode) {
            continue;
        }
        let Ok(path_str) = std::str::from_utf8(entry.path(&index)) else {
            continue;
        };
        if is_in_excluded_dir(path_str) || is_excluded_filename(path_str) {
            continue;
        }
        // Count toward the I/O budget only here — we want the cap to bound real reads, not
        // index entries skipped by the cheap exclusion checks. Otherwise a committed
        // `target/` or `node_modules/` (5k+ entries each) can exhaust the budget before
        // visiting any real source.
        scanned += 1;
        let abs = workdir.join(path_str);
        let Ok(metadata) = std::fs::metadata(&abs) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > FILE_SIZE_CAP {
            continue;
        }
        let Ok(bytes) = std::fs::read(&abs) else {
            continue;
        };
        if looks_binary(&bytes) {
            continue;
        }
        visit(path_str, &bytes);
    }
    Ok(())
}

fn is_regular_file(mode: gix::index::entry::Mode) -> bool {
    use gix::index::entry::Mode;
    matches!(mode, Mode::FILE | Mode::FILE_EXECUTABLE)
}

/// True iff any slash-separated segment of `path` matches a name in [`EXCLUDED_DIRS`].
pub fn is_in_excluded_dir(path: &str) -> bool {
    path.split('/').any(|seg| EXCLUDED_DIRS.contains(&seg))
}

/// True iff `path`'s basename matches a name in [`EXCLUDED_FILENAMES`].
pub fn is_excluded_filename(path: &str) -> bool {
    let basename = path.rsplit('/').next().unwrap_or(path);
    EXCLUDED_FILENAMES.contains(&basename)
}

pub fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    head.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_dir_check_only_matches_segments() {
        // `vendor/x.go` excluded; `myvendor/x.go` and `vendor.go` are regular files.
        assert!(is_in_excluded_dir("vendor/foo.go"));
        assert!(is_in_excluded_dir("src/vendor/foo.go"));
        assert!(is_in_excluded_dir("a/b/node_modules/c"));
        assert!(!is_in_excluded_dir("myvendor/foo.go"));
        assert!(!is_in_excluded_dir("src/vendor.go"));
        assert!(!is_in_excluded_dir("README.md"));
    }

    #[test]
    fn excluded_filename_matches_basename_anywhere_in_tree() {
        assert!(is_excluded_filename("Cargo.lock"));
        assert!(is_excluded_filename("crates/foo/Cargo.lock"));
        assert!(is_excluded_filename("frontend/package-lock.json"));
        assert!(is_excluded_filename("vendor/foo/go.sum"));
        // Substring / prefix matches don't count — only exact basename.
        assert!(!is_excluded_filename("Cargo.lock.bak"));
        assert!(!is_excluded_filename("not-package-lock.json"));
        assert!(!is_excluded_filename("README.md"));
    }

    #[test]
    fn looks_binary_detects_nul_byte_in_head() {
        assert!(looks_binary(b"\x00\x01\x02"));
        assert!(looks_binary(b"text\x00more"));
        assert!(!looks_binary(b"plain ascii"));
        assert!(!looks_binary(b""));
    }
}
