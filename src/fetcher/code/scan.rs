//! Shared index walker for the `code_*` family. Iterates tracked files in the repo discovered
//! from CWD, skipping committed-vendored trees, generated outputs, machine-written lock files,
//! oversized files, and binaries. Each surviving file's bytes are handed to the visitor.
//!
//! Untracked / `.gitignore`-d files are filtered for free by walking the index. The
//! [`EXCLUDED_DIRS`] and [`EXCLUDED_FILENAMES`] lists are the second line of defense for
//! noise that's been committed anyway and would otherwise inflate metrics
//! (`Cargo.lock` alone is ~6k lines on a typical Rust repo).

use tokei::{CodeStats, Config, LanguageType};

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

/// Visit each tracked path in the repo's index, applying the same exclusion / regular-file
/// filters as [`for_each_tracked_file`] but **without** reading file contents. Right for
/// fetchers that only need extension-level classification (`code_language_logo` picks the
/// dominant language by file count alone — no need to read every blob to count its lines).
pub fn for_each_tracked_path<F>(repo: &gix::Repository, mut visit: F) -> Result<(), FetchError>
where
    F: FnMut(&str),
{
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
        scanned += 1;
        visit(path_str);
    }
    Ok(())
}

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

/// Path → display label + tokei [`LanguageType`]. Pre-checks bare filenames tokei misses
/// (`Gemfile`, `Brewfile` → Ruby), then defers to tokei's extension table. Returns the
/// canonicalised display name (`TSX` / `JSX` are folded into `TypeScript` / `JavaScript`
/// for splash purposes — they're variant syntaxes of the same language for the user's
/// mental model).
pub fn classify_with_lang(path: &str, cfg: &Config) -> Option<(&'static str, LanguageType)> {
    if let Some(forced) = special_basename(path) {
        return Some(forced);
    }
    let lang = LanguageType::from_path(path, cfg)?;
    Some((canonical_name(lang.name()), lang))
}

/// Display-name-only convenience for path-only callers (`code_language_logo`).
pub fn classify_path(path: &str) -> Option<&'static str> {
    classify_with_lang(path, &Config::default()).map(|(name, _)| name)
}

fn special_basename(path: &str) -> Option<(&'static str, LanguageType)> {
    let basename = path.rsplit('/').next().unwrap_or(path);
    match basename {
        "Gemfile" | "Brewfile" => Some(("Ruby", LanguageType::Ruby)),
        _ => None,
    }
}

fn canonical_name(name: &'static str) -> &'static str {
    match name {
        "TSX" => "TypeScript",
        "JSX" => "JavaScript",
        _ => name,
    }
}

/// Visit each tracked, classifiable, in-bounds file with its display label and per-file
/// [`CodeStats`] (`code` / `comments` / `blanks`). Files whose extension neither tokei nor
/// [`special_basename`] recognise are skipped silently; aggregate them under "Other"
/// upstream if needed. Reuses [`for_each_tracked_file`]'s exclusion + size-cap rules.
pub fn for_each_tokei_stat<F>(repo: &gix::Repository, mut visit: F) -> Result<(), FetchError>
where
    F: FnMut(&str, &'static str, &CodeStats),
{
    let cfg = Config::default();
    for_each_tracked_file(repo, |path, bytes| {
        if let Some((name, lang)) = classify_with_lang(path, &cfg) {
            let stats = lang.parse_from_slice(bytes, &cfg);
            visit(path, name, &stats);
        }
    })
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    use super::super::super::git::test_support::make_repo;
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git should be on PATH for tests");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write_file(dir: &Path, rel: &str, bytes: impl AsRef<[u8]>) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

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

    #[test]
    fn classify_helpers_cover_special_basenames_and_canonical_variants() {
        let cfg = Config::default();

        assert_eq!(
            classify_with_lang("Gemfile", &cfg),
            Some(("Ruby", LanguageType::Ruby))
        );
        assert_eq!(
            classify_with_lang("Brewfile", &cfg).map(|(name, _)| name),
            Some("Ruby")
        );
        assert_eq!(classify_path("src/app.tsx"), Some("TypeScript"));
        assert_eq!(classify_path("src/app.jsx"), Some("JavaScript"));
        assert_eq!(classify_path("notes.unknown-extension"), None);
    }

    #[test]
    fn tracked_file_walker_is_noop_for_bare_repo() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "--bare", "-q"]);
        let repo = gix::open(tmp.path()).unwrap();
        let mut visited = false;

        for_each_tracked_file(&repo, |_path, _bytes| visited = true).unwrap();

        assert!(!visited);
    }

    #[test]
    fn tracked_walkers_skip_excluded_missing_large_binary_and_unreadable_files() {
        let (_tmp, repo) = make_repo();
        let dir = repo.workdir().unwrap();
        write_file(dir, "src/main.rs", "fn main() {}\n");
        write_file(dir, "vendor/ignored.rs", "pub fn ignored() {}\n");
        write_file(dir, "Cargo.lock", "noise");
        write_file(dir, "Gemfile", "puts 'hi'\n");
        write_file(dir, "plain.txt", "hello\n");
        write_file(dir, "missing.txt", "soon gone\n");
        write_file(dir, "large.txt", vec![b'x'; FILE_SIZE_CAP as usize + 1]);
        write_file(dir, "binary.dat", b"abc\0def");
        write_file(dir, "unreadable.txt", "secret\n");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "fixture"]);

        std::fs::remove_file(dir.join("missing.txt")).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(
            dir.join("unreadable.txt"),
            std::fs::Permissions::from_mode(0o000),
        )
        .unwrap();

        let mut path_hits = Vec::new();
        for_each_tracked_path(&repo, |path| path_hits.push(path.to_string())).unwrap();
        path_hits.sort();

        assert!(path_hits.iter().any(|path| path == "src/main.rs"));
        assert!(path_hits.iter().any(|path| path == "Gemfile"));
        assert!(path_hits.iter().any(|path| path == "plain.txt"));
        assert!(path_hits.iter().any(|path| path == "missing.txt"));
        assert!(path_hits.iter().any(|path| path == "large.txt"));
        assert!(path_hits.iter().any(|path| path == "binary.dat"));
        assert!(path_hits.iter().any(|path| path == "unreadable.txt"));
        assert!(!path_hits.iter().any(|path| path == "vendor/ignored.rs"));
        assert!(!path_hits.iter().any(|path| path == "Cargo.lock"));

        let mut file_hits = Vec::new();
        for_each_tracked_file(&repo, |path, _bytes| file_hits.push(path.to_string())).unwrap();
        file_hits.sort();

        assert!(file_hits.iter().any(|path| path == "src/main.rs"));
        assert!(file_hits.iter().any(|path| path == "Gemfile"));
        assert!(file_hits.iter().any(|path| path == "plain.txt"));
        assert!(!file_hits.iter().any(|path| path == "missing.txt"));
        assert!(!file_hits.iter().any(|path| path == "large.txt"));
        assert!(!file_hits.iter().any(|path| path == "binary.dat"));
        #[cfg(unix)]
        assert!(!file_hits.iter().any(|path| path == "unreadable.txt"));
        assert!(!file_hits.iter().any(|path| path == "vendor/ignored.rs"));
        assert!(!file_hits.iter().any(|path| path == "Cargo.lock"));

        #[cfg(unix)]
        std::fs::set_permissions(
            dir.join("unreadable.txt"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }

    #[test]
    fn tokei_stat_walker_reports_classified_files_only() {
        let (_tmp, repo) = make_repo();
        let dir = repo.workdir().unwrap();
        write_file(dir, "Gemfile", "puts 'hi'\n");
        write_file(dir, "src/lib.rs", "fn answer() -> u32 {\n    42\n}\n");
        write_file(dir, "notes.unknown-extension", "ignored\n");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-q", "-m", "fixture"]);

        let mut stats = Vec::new();
        for_each_tokei_stat(&repo, |path, name, stat| {
            stats.push((path.to_string(), name, stat.code));
        })
        .unwrap();
        stats.sort();

        assert_eq!(stats.len(), 2);
        assert_eq!(stats[0], ("Gemfile".into(), "Ruby", 1));
        assert_eq!(stats[1], ("src/lib.rs".into(), "Rust", 3));
    }
}
