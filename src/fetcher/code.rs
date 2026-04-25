//! Code-internal metric fetchers — grep / parse the source tree of the discovered git repo.
//! All `Safety::Safe` (local file reads only).
//!
//! Distinct from the future `project_*` family in #62, which is for project-level operational
//! state (test coverage, bundle size, dependency alerts, license check). `code_*` is "what's
//! inside the source files".

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{Bar, BarsData, Body, EntriesData, Entry, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

use super::git::{fail, open_repo, payload, repo_cache_key};
use super::{FetchContext, FetchError, Fetcher, Safety};

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(CodeTodos)]
}

const SHAPES: &[Shape] = &[Shape::Text, Shape::TextBlock, Shape::Entries, Shape::Bars];
const DEFAULT_MARKERS: &[&str] = &["TODO", "FIXME"];
const DEFAULT_LIMIT: usize = 20;
const FILE_SIZE_CAP: u64 = 1024 * 1024;
const MAX_FILES_SCANNED: usize = 5_000;
const INTERNAL_HIT_CAP: usize = 500;
const SNIPPET_CHARS: usize = 100;

/// Path-segment prefixes that get skipped even when they're tracked. Covers the common case
/// of vendored / generated trees that some projects commit (Go monorepos with `vendor/`,
/// repos that ship a `dist/` build, accidental `node_modules/`). `.gitignore`-d trees are
/// already filtered out by walking the index — this list is the second line of defense for
/// committed-vendored code that would otherwise inflate hit counts with library `TODO`s.
const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "third_party",
    "target",
    "dist",
    "build",
    ".git",
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "markers",
        type_hint: "array of strings",
        required: false,
        default: Some("[\"TODO\", \"FIXME\"]"),
        description: "Markers grepped for. Each must appear as a whole-word ASCII token followed by `:` (or `(name):`) — `// TODO: refactor` matches, `## TODO` heading and `\"TODO\"` literals don't.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("20"),
        description: "Cap on rendered hits (TextBlock) or rows (Entries / Bars). Total count surfaced by `Text` is unaffected.",
    },
];

/// Greps tracked source files for `TODO:` / `FIXME:`-style comment markers. The trailing
/// colon (also accepting `TODO(name):`) is required so README headings, prose, and string
/// literals don't inflate the count. `Text` summarises `"N TODOs in M files"`; `TextBlock`
/// lists `path:line: snippet`; `Entries` / `Bars` rank files by hit count. Walks the index
/// of the repo discovered from CWD so untracked and `.gitignore`-d files are skipped
/// automatically; vendored / generated trees that happen to be committed (`node_modules/`,
/// `vendor/`, `dist/`, `target/`, …) are skipped via [`EXCLUDED_DIRS`] so library TODOs
/// don't bleed into the count.
pub struct CodeTodos;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    markers: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Fetcher for CodeTodos {
    fn name(&self) -> &str {
        "code_todos"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Mix `[widget.options]` into the key — markers / limit change what gets read, so two
        // widgets pointing at this fetcher with different options must not share a cache slot.
        // `toml = "0.8"` keeps tables in sorted order, so `Value::to_string()` is stable across runs.
        let base = repo_cache_key(self.name(), ctx);
        let opts = ctx
            .options
            .as_ref()
            .map(toml::Value::to_string)
            .unwrap_or_default();
        if opts.is_empty() {
            return base;
        }
        let digest = Sha256::digest(opts.as_bytes());
        let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
        format!("{base}-{hex}")
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("12 TODOs in 5 files"),
            Shape::TextBlock => samples::text_block(&[
                "src/render/mod.rs:120: TODO: handle empty body",
                "src/fetcher/git.rs:88: FIXME: don't panic on detached head",
                "src/main.rs:14: TODO: load config from $HOME",
            ]),
            Shape::Entries => samples::entries(&[
                ("src/render/mod.rs", "5"),
                ("src/fetcher/git.rs", "3"),
                ("src/main.rs", "2"),
            ]),
            Shape::Bars => samples::bars(&[
                ("src/render/mod.rs", 5),
                ("src/fetcher/git.rs", 3),
                ("src/main.rs", 2),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let markers = resolve_markers(opts.markers.as_deref());
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let scan = scan_cwd(&markers)?;
        Ok(payload(render_body(
            scan,
            ctx.shape.unwrap_or(Shape::Text),
            limit,
        )))
    }
}

fn parse_options(raw: Option<&toml::Value>) -> Result<Options, FetchError> {
    match raw {
        None => Ok(Options::default()),
        Some(value) => value
            .clone()
            .try_into::<Options>()
            .map_err(|e| FetchError::Failed(format!("invalid options: {e}"))),
    }
}

fn resolve_markers(opt: Option<&[String]>) -> Vec<String> {
    let custom: Vec<String> = opt
        .map(|v| {
            v.iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if custom.is_empty() {
        DEFAULT_MARKERS.iter().map(|s| (*s).into()).collect()
    } else {
        custom
    }
}

#[derive(Debug, Default)]
struct ScanResult {
    hits: Vec<Hit>,
    total: usize,
    files_with_hits: usize,
    file_counts: Vec<(String, u64)>,
}

#[derive(Debug)]
struct Hit {
    path: String,
    line: usize,
    text: String,
}

fn scan_cwd(markers: &[String]) -> Result<ScanResult, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo, markers)
}

fn scan_repo(repo: &gix::Repository, markers: &[String]) -> Result<ScanResult, FetchError> {
    let Some(workdir) = repo.workdir().map(|p| p.to_path_buf()) else {
        return Ok(ScanResult::default());
    };
    let index = match repo.index_or_load_from_head_or_empty() {
        Ok(i) => i,
        Err(e) => return Err(fail(e)),
    };
    let needles: Vec<&str> = markers.iter().map(String::as_str).collect();
    let mut hits = Vec::new();
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0usize;
    let mut scanned = 0usize;
    for entry in index.entries() {
        if scanned >= MAX_FILES_SCANNED {
            break;
        }
        if !is_regular_file(entry.mode) {
            continue;
        }
        scanned += 1;
        let Ok(path_str) = std::str::from_utf8(entry.path(&index)) else {
            continue;
        };
        if is_in_excluded_dir(path_str) {
            continue;
        }
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
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            if let Some(snippet) = match_marker(line, &needles) {
                total += 1;
                *counts.entry(path_str.to_string()).or_insert(0) += 1;
                if hits.len() < INTERNAL_HIT_CAP {
                    hits.push(Hit {
                        path: path_str.to_string(),
                        line: idx + 1,
                        text: snippet,
                    });
                }
            }
        }
    }
    let files_with_hits = counts.len();
    let mut file_counts: Vec<_> = counts.into_iter().collect();
    file_counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(ScanResult {
        hits,
        total,
        files_with_hits,
        file_counts,
    })
}

fn is_regular_file(mode: gix::index::entry::Mode) -> bool {
    use gix::index::entry::Mode;
    matches!(mode, Mode::FILE | Mode::FILE_EXECUTABLE)
}

/// True iff any segment of `path` (slash-separated, as the index always stores it) matches a
/// directory name in [`EXCLUDED_DIRS`]. Skips committed-vendored trees so library TODOs don't
/// inflate the count.
fn is_in_excluded_dir(path: &str) -> bool {
    path.split('/').any(|seg| EXCLUDED_DIRS.contains(&seg))
}

fn looks_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    head.contains(&0)
}

fn match_marker(line: &str, markers: &[&str]) -> Option<String> {
    let bytes = line.as_bytes();
    let mut best: Option<usize> = None;
    for needle in markers {
        if let Some(pos) = find_marker(bytes, needle.as_bytes())
            && best.map(|b| pos < b).unwrap_or(true)
        {
            best = Some(pos);
        }
    }
    best.map(|pos| truncate(line[pos..].trim_end(), SNIPPET_CHARS))
}

/// Find a marker followed by a `TODO:` / `TODO(name):` style colon. Word-boundary on the
/// leading side so `TODOLIST` doesn't match; trailing `:` excludes README headings, prose,
/// and string-literal hits like `"TODO"`. The optional `(...)` arm handles the `TODO(yuji):`
/// author-tag convention. `::` (C++ namespace) is rejected explicitly.
fn find_marker(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| {
        let end = i + needle.len();
        haystack[i..end] == *needle
            && (i == 0 || !is_word_byte(haystack[i - 1]))
            && (end < haystack.len() && !is_word_byte(haystack[end]))
            && trailing_colon(haystack, end)
    })
}

fn trailing_colon(bytes: &[u8], mut i: usize) -> bool {
    if bytes.get(i) == Some(&b'(') {
        let mut depth = 1;
        i += 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return false;
        }
    }
    bytes.get(i) == Some(&b':') && bytes.get(i + 1) != Some(&b':')
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn render_body(scan: ScanResult, shape: Shape, limit: usize) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: scan
                .hits
                .into_iter()
                .take(limit)
                .map(|h| format!("{}:{}: {}", h.path, h.line, h.text))
                .collect(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: scan
                .file_counts
                .into_iter()
                .take(limit)
                .map(|(p, c)| Entry {
                    key: p,
                    value: Some(c.to_string()),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: scan
                .file_counts
                .into_iter()
                .take(limit)
                .map(|(p, c)| Bar { label: p, value: c })
                .collect(),
        }),
        _ => text_summary(scan.total, scan.files_with_hits),
    }
}

fn text_summary(total: usize, files: usize) -> Body {
    let value = if total == 0 {
        String::new()
    } else {
        format!(
            "{total} TODO{} in {files} file{}",
            if total == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" },
        )
    };
    Body::Text(TextData { value })
}

#[cfg(test)]
mod tests {
    use super::super::git::test_support::{commit_touching, make_repo};
    use super::*;

    fn defaults() -> Vec<String> {
        DEFAULT_MARKERS.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn empty_repo_returns_empty_scan() {
        let (_tmp, repo) = make_repo();
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 0);
        assert!(scan.hits.is_empty());
    }

    #[test]
    fn picks_up_marker_in_tracked_file() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "// TODO: refactor this");
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 1);
        let hit = scan.hits.first().expect("one hit");
        assert_eq!(hit.path, "src/main.rs");
        assert!(hit.text.starts_with("TODO"));
    }

    #[test]
    fn marker_without_trailing_colon_is_ignored() {
        // README headings, prose, string literals: noise we explicitly want to filter out.
        let (_tmp, repo) = make_repo();
        commit_touching(
            &repo,
            "README.md",
            "## TODO\nstill TODO somewhere\nlet x = \"TODO\";",
        );
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 0);
    }

    #[test]
    fn author_tagged_marker_is_recognised() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/lib.rs", "// TODO(yuji): wire this up");
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 1);
        assert!(scan.hits[0].text.starts_with("TODO(yuji):"));
    }

    #[test]
    fn untracked_file_is_ignored() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "tracked.rs", "nothing");
        let workdir = repo.workdir().unwrap();
        std::fs::write(workdir.join("untracked.rs"), "// TODO: should be skipped\n").unwrap();
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 0);
    }

    #[test]
    fn ranks_files_by_hit_count() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "many.rs", "// TODO: a\n// TODO: b\n// TODO: c");
        commit_touching(&repo, "few.rs", "// TODO: only one");
        let scan = scan_repo(&repo, &defaults()).unwrap();
        let labels: Vec<_> = scan.file_counts.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(labels, vec!["many.rs", "few.rs"]);
        assert_eq!(scan.file_counts[0].1, 3);
        assert_eq!(scan.files_with_hits, 2);
    }

    #[test]
    fn matcher_requires_word_boundary_and_trailing_colon() {
        // Word boundary: `TODOLIST` must not match.
        assert!(match_marker("let TODOLIST = 1;", &["TODO"]).is_none());
        // Trailing colon: prose / headings / literals without `:` must not match.
        assert!(match_marker("## TODO", &["TODO"]).is_none());
        assert!(match_marker("/*TODO*/", &["TODO"]).is_none());
        assert!(match_marker("let x = \"TODO\";", &["TODO"]).is_none());
        // Canonical comment forms match.
        assert!(match_marker("// TODO: x", &["TODO"]).is_some());
        assert!(match_marker("# TODO: x", &["TODO"]).is_some());
        assert!(match_marker("// TODO(yuji): x", &["TODO"]).is_some());
        // C++ namespace `std::TODO::foo` must not match.
        assert!(match_marker("std::TODO::foo()", &["TODO"]).is_none());
    }

    #[test]
    fn matcher_picks_first_marker_in_line() {
        // Snippet starts at the marker, dropping any preceding comment chars.
        let snippet = match_marker("    // TODO: real text   ", &["TODO", "FIXME"]).unwrap();
        assert_eq!(snippet, "TODO: real text");
    }

    #[test]
    fn binary_files_are_skipped() {
        let (tmp, repo) = make_repo();
        let workdir = tmp.path();
        std::fs::write(workdir.join("blob.bin"), b"\x00\x01\x02TODO: x\x00").unwrap();
        // Track and commit the binary so the index sees it.
        std::process::Command::new("git")
            .args(["add", "blob.bin"])
            .current_dir(workdir)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-q", "-m", "bin"])
            .current_dir(workdir)
            .status()
            .unwrap();
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 0);
    }

    #[test]
    fn vendored_directories_are_excluded_even_when_committed() {
        // `commit_touching` reuses the message as the appended file content, so we use
        // distinct payloads with the marker baked into each.
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "// TODO: app marker");
        commit_touching(&repo, "node_modules/foo.js", "// TODO: node marker");
        commit_touching(&repo, "vendor/lib/x.go", "// TODO: vendor marker");
        commit_touching(&repo, "src/dist/x.js", "// TODO: dist marker");
        let scan = scan_repo(&repo, &defaults()).unwrap();
        assert_eq!(scan.total, 1);
        assert_eq!(scan.hits[0].path, "src/main.rs");
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
    fn custom_markers_supersede_defaults() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "x.rs", "// TODO: ignored\n// XXX: caught");
        let markers = vec!["XXX".to_string()];
        let scan = scan_repo(&repo, &markers).unwrap();
        assert_eq!(scan.total, 1);
        assert!(scan.hits[0].text.starts_with("XXX:"));
    }

    #[test]
    fn text_summary_reports_total_and_file_count() {
        let body = render_body(
            ScanResult {
                hits: vec![],
                total: 3,
                files_with_hits: 2,
                file_counts: vec![("a".into(), 2), ("b".into(), 1)],
            },
            Shape::Text,
            10,
        );
        match body {
            Body::Text(d) => assert_eq!(d.value, "3 TODOs in 2 files"),
            _ => panic!(),
        }
    }

    #[test]
    fn text_summary_handles_singular_grammar() {
        let body = render_body(
            ScanResult {
                hits: vec![],
                total: 1,
                files_with_hits: 1,
                file_counts: vec![("a".into(), 1)],
            },
            Shape::Text,
            10,
        );
        match body {
            Body::Text(d) => assert_eq!(d.value, "1 TODO in 1 file"),
            _ => panic!(),
        }
    }

    #[test]
    fn text_summary_is_empty_when_no_hits() {
        let body = render_body(ScanResult::default(), Shape::Text, 10);
        match body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_formats_path_line_snippet() {
        let body = render_body(
            ScanResult {
                hits: vec![Hit {
                    path: "src/main.rs".into(),
                    line: 12,
                    text: "TODO: x".into(),
                }],
                total: 1,
                files_with_hits: 1,
                file_counts: vec![("src/main.rs".into(), 1)],
            },
            Shape::TextBlock,
            10,
        );
        match body {
            Body::TextBlock(d) => assert_eq!(d.lines, vec!["src/main.rs:12: TODO: x"]),
            _ => panic!(),
        }
    }

    #[test]
    fn limit_caps_rendered_rows() {
        let scan = ScanResult {
            hits: vec![],
            total: 5,
            files_with_hits: 5,
            file_counts: (0..5).map(|i| (format!("f{i}.rs"), 1)).collect(),
        };
        let body = render_body(scan, Shape::Entries, 2);
        match body {
            Body::Entries(d) => assert_eq!(d.items.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn truncate_appends_ellipsis_when_over_max() {
        let s = "x".repeat(120);
        let out = truncate(&s, 100);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 101);
    }

    #[test]
    fn cache_key_changes_with_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let key_default = CodeTodos.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"markers = ["TODO"]"#).unwrap());
        let key_a = CodeTodos.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"markers = ["FIXME"]"#).unwrap());
        let key_b = CodeTodos.cache_key(&ctx);
        assert_ne!(key_default, key_a);
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn cache_key_is_stable_for_equivalent_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        ctx.options = Some(toml::from_str(r#"markers = ["TODO", "FIXME"]"#).unwrap());
        let a = CodeTodos.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"markers = ["TODO", "FIXME"]"#).unwrap());
        let b = CodeTodos.cache_key(&ctx);
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_markers_falls_back_when_empty() {
        let empty: Vec<String> = Vec::new();
        let blanks: Vec<String> = vec!["  ".into(), "".into()];
        let custom: Vec<String> = vec!["XXX".into()];
        assert_eq!(resolve_markers(None), defaults());
        assert_eq!(resolve_markers(Some(&empty)), defaults());
        assert_eq!(resolve_markers(Some(&blanks)), defaults());
        assert_eq!(resolve_markers(Some(&custom)), vec!["XXX".to_string()]);
    }
}
