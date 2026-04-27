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
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, NumberSeriesData,
    Payload, RatioData, Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::git::{fail, open_repo, payload, repo_cache_key};
use super::{FetchContext, FetchError, Fetcher, Safety};

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(CodeTodos), Arc::new(CodeLanguages)]
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
    fn description(&self) -> &'static str {
        "Greps tracked source files in the discovered git repo for `TODO:` / `FIXME:` style markers (trailing colon required, vendored / generated dirs skipped). `Text` summarises `\"N TODOs in M files\"`, `TextBlock` lists `path:line: snippet`, and `Entries` / `Bars` rank files by hit count."
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

fn parse_options<T: Default + serde::de::DeserializeOwned>(
    raw: Option<&toml::Value>,
) -> Result<T, FetchError> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
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

const LANG_SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::Bars,
    Shape::Badge,
];
const DEFAULT_LANG_LIMIT: usize = 10;
const LANG_TEXT_TOP_N: usize = 3;
const OTHER_LABEL: &str = "Other";

/// Extension → display language. Lowercased ASCII keys; lookup lowercases the file extension
/// before matching. Filename-only conventions (`Dockerfile`, `Makefile`, `CMakeLists.txt`) are
/// handled separately by [`match_filename`].
const EXTENSION_TO_LANG: &[(&str, &str)] = &[
    ("rs", "Rust"),
    ("go", "Go"),
    ("c", "C"),
    ("h", "C"),
    ("cc", "C++"),
    ("cpp", "C++"),
    ("cxx", "C++"),
    ("hpp", "C++"),
    ("hh", "C++"),
    ("zig", "Zig"),
    ("nim", "Nim"),
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("kts", "Kotlin"),
    ("scala", "Scala"),
    ("clj", "Clojure"),
    ("groovy", "Groovy"),
    ("swift", "Swift"),
    ("m", "Objective-C"),
    ("mm", "Objective-C"),
    ("cs", "C#"),
    ("fs", "F#"),
    ("vb", "VB.NET"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("js", "JavaScript"),
    ("jsx", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("html", "HTML"),
    ("htm", "HTML"),
    ("css", "CSS"),
    ("scss", "SCSS"),
    ("sass", "SCSS"),
    ("less", "Less"),
    ("vue", "Vue"),
    ("svelte", "Svelte"),
    ("astro", "Astro"),
    ("py", "Python"),
    ("pyi", "Python"),
    ("rb", "Ruby"),
    ("php", "PHP"),
    ("pl", "Perl"),
    ("pm", "Perl"),
    ("lua", "Lua"),
    ("r", "R"),
    ("hs", "Haskell"),
    ("ml", "OCaml"),
    ("mli", "OCaml"),
    ("ex", "Elixir"),
    ("exs", "Elixir"),
    ("erl", "Erlang"),
    ("elm", "Elm"),
    ("sh", "Shell"),
    ("bash", "Shell"),
    ("zsh", "Shell"),
    ("fish", "Shell"),
    ("ps1", "PowerShell"),
    ("dart", "Dart"),
    ("md", "Markdown"),
    ("markdown", "Markdown"),
    ("rst", "reStructuredText"),
    ("tex", "LaTeX"),
    ("toml", "TOML"),
    ("yaml", "YAML"),
    ("yml", "YAML"),
    ("json", "JSON"),
    ("xml", "XML"),
    ("sql", "SQL"),
    ("proto", "Protobuf"),
    ("graphql", "GraphQL"),
    ("gql", "GraphQL"),
    ("nix", "Nix"),
    ("tf", "Terraform"),
    ("hcl", "HCL"),
    ("mk", "Makefile"),
    ("cmake", "CMake"),
    ("gradle", "Gradle"),
    ("vim", "Vim script"),
    ("el", "Emacs Lisp"),
    ("scm", "Scheme"),
    ("lisp", "Common Lisp"),
    ("jl", "Julia"),
    ("rkt", "Racket"),
];

const LANG_OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("10"),
        description: "Top-N languages kept after sorting by `unit`. Tail (when present) is collapsed into a single `Other` row when `include_others = true`.",
    },
    OptionSchema {
        name: "unit",
        type_hint: "\"lines\" | \"files\"",
        required: false,
        default: Some("\"lines\""),
        description: "What to sort + size by. `lines` weights long files (better for language-mix at a glance); `files` weights file count (closer to project-structure feel).",
    },
    OptionSchema {
        name: "include_others",
        type_hint: "boolean",
        required: false,
        default: Some("true"),
        description: "When the language count exceeds `limit`, append a synthetic `Other` row aggregating the tail. Disable to hard-cap the list.",
    },
];

/// Walks the discovered git index and groups tracked source files by language (extension table
/// plus a few filename conventions). `Bars` / `Entries` rank languages by `lines` (default) or
/// `files`; `Text` summarises the top 3 as `"Rust 67% · TypeScript 22% · Python 11%"`;
/// `TextBlock` / `MarkdownTextBlock` list percentages line-by-line; `Ratio` exposes the dominant
/// language's share; `NumberSeries` is the top-N values for sparkline use; `Badge` returns the
/// dominant language as a label. Same `EXCLUDED_DIRS` filter as `code_todos` so vendored /
/// generated trees don't bias the count.
pub struct CodeLanguages;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LangOptions {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    unit: Option<LangUnit>,
    #[serde(default)]
    include_others: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum LangUnit {
    #[default]
    Lines,
    Files,
}

impl LangUnit {
    fn label(self) -> &'static str {
        match self {
            Self::Lines => "lines",
            Self::Files => "files",
        }
    }
}

#[async_trait]
impl Fetcher for CodeLanguages {
    fn name(&self) -> &str {
        "code_languages"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Groups tracked source files in the discovered git repo by language and ranks them by lines (default) or files. `Bars` / `Entries` are the structural shapes; `Text` summarises the top 3, `TextBlock` / `MarkdownTextBlock` list percentages line-by-line, `Ratio` exposes the dominant language's share, `NumberSeries` is the top-N values for sparkline use, and `Badge` returns the dominant language as a label."
    }
    fn shapes(&self) -> &[Shape] {
        LANG_SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Bars
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
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
        LANG_OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("Rust 67% · TypeScript 22% · Python 11%"),
            Shape::TextBlock => samples::text_block(&[
                "Rust            67%   12,345 lines",
                "TypeScript      22%    4,123 lines",
                "Python          11%    2,001 lines",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **Rust** — 67% · 12,345 lines\n- TypeScript — 22% · 4,123 lines\n- Python — 11% · 2,001 lines",
            ),
            Shape::Entries => samples::entries(&[
                ("Rust", "67% (12,345 lines)"),
                ("TypeScript", "22% (4,123 lines)"),
                ("Python", "11% (2,001 lines)"),
            ]),
            Shape::Ratio => samples::ratio(0.67, "Rust"),
            Shape::NumberSeries => samples::number_series(&[12_345, 4_123, 2_001, 480, 120]),
            Shape::Bars => {
                samples::bars(&[("Rust", 12_345), ("TypeScript", 4_123), ("Python", 2_001)])
            }
            Shape::Badge => samples::badge(Status::Ok, "Rust"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: LangOptions = parse_options(ctx.options.as_ref())?;
        let unit = opts.unit.unwrap_or_default();
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LANG_LIMIT);
        let include_others = opts.include_others.unwrap_or(true);
        let scan = scan_languages_cwd(unit)?;
        let entries = apply_lang_limit(scan.entries, limit, include_others);
        Ok(payload(render_lang_body(
            entries,
            ctx.shape.unwrap_or(Shape::Bars),
            unit,
        )))
    }
}

#[derive(Debug, Default, Clone)]
struct LangScanResult {
    /// Sorted descending by the chosen unit. Tail is intact (limit is applied later).
    entries: Vec<LangEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LangEntry {
    lang: String,
    lines: u64,
    files: u64,
}

fn scan_languages_cwd(unit: LangUnit) -> Result<LangScanResult, FetchError> {
    let repo = open_repo()?;
    scan_languages_repo(&repo, unit)
}

fn scan_languages_repo(
    repo: &gix::Repository,
    unit: LangUnit,
) -> Result<LangScanResult, FetchError> {
    let Some(workdir) = repo.workdir().map(|p| p.to_path_buf()) else {
        return Ok(LangScanResult::default());
    };
    let index = repo.index_or_load_from_head_or_empty().map_err(fail)?;
    let mut by_lang: HashMap<&'static str, (u64, u64)> = HashMap::new();
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
        let Some(lang) = detect_language(path_str) else {
            continue;
        };
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
        let lines = count_lines(&bytes);
        let slot = by_lang.entry(lang).or_insert((0, 0));
        slot.0 += lines;
        slot.1 += 1;
    }
    let mut entries: Vec<LangEntry> = by_lang
        .into_iter()
        .map(|(lang, (lines, files))| LangEntry {
            lang: lang.to_string(),
            lines,
            files,
        })
        .collect();
    entries.sort_by(|a, b| {
        lang_metric(b, unit)
            .cmp(&lang_metric(a, unit))
            .then_with(|| a.lang.cmp(&b.lang))
    });
    Ok(LangScanResult { entries })
}

fn detect_language(path: &str) -> Option<&'static str> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    if let Some(lang) = match_filename(filename) {
        return Some(lang);
    }
    let (stem, ext) = filename.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let ext_lower = ext.to_ascii_lowercase();
    EXTENSION_TO_LANG
        .iter()
        .find(|(e, _)| *e == ext_lower)
        .map(|(_, l)| *l)
}

fn match_filename(name: &str) -> Option<&'static str> {
    match name {
        "Dockerfile" | "dockerfile" | "Containerfile" => Some("Dockerfile"),
        "Makefile" | "makefile" | "GNUmakefile" => Some("Makefile"),
        "CMakeLists.txt" => Some("CMake"),
        "Rakefile" => Some("Ruby"),
        "Gemfile" => Some("Ruby"),
        _ => None,
    }
}

fn count_lines(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let nls = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    if bytes.last() == Some(&b'\n') {
        nls
    } else {
        nls + 1
    }
}

fn lang_metric(e: &LangEntry, unit: LangUnit) -> u64 {
    match unit {
        LangUnit::Lines => e.lines,
        LangUnit::Files => e.files,
    }
}

fn lang_total(entries: &[LangEntry], unit: LangUnit) -> u64 {
    entries.iter().map(|e| lang_metric(e, unit)).sum()
}

fn apply_lang_limit(
    mut entries: Vec<LangEntry>,
    limit: usize,
    include_others: bool,
) -> Vec<LangEntry> {
    if entries.len() <= limit {
        return entries;
    }
    let tail = entries.split_off(limit);
    if include_others && !tail.is_empty() {
        let lines = tail.iter().map(|e| e.lines).sum();
        let files = tail.iter().map(|e| e.files).sum();
        entries.push(LangEntry {
            lang: OTHER_LABEL.to_string(),
            lines,
            files,
        });
    }
    entries
}

fn render_lang_body(entries: Vec<LangEntry>, shape: Shape, unit: LangUnit) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: format_lang_textblock(&entries, unit),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format_lang_markdown(&entries, unit),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: format_lang_entries(&entries, unit),
        }),
        Shape::Ratio => format_lang_ratio(&entries, unit),
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData {
            values: entries.iter().map(|e| lang_metric(e, unit)).collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: entries
                .iter()
                .map(|e| Bar {
                    label: e.lang.clone(),
                    value: lang_metric(e, unit),
                })
                .collect(),
        }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Ok,
            label: entries.first().map(|e| e.lang.clone()).unwrap_or_default(),
        }),
        _ => Body::Text(TextData {
            value: format_lang_text(&entries, unit),
        }),
    }
}

fn format_lang_text(entries: &[LangEntry], unit: LangUnit) -> String {
    let total = lang_total(entries, unit);
    if total == 0 {
        return String::new();
    }
    entries
        .iter()
        .take(LANG_TEXT_TOP_N)
        .map(|e| format!("{} {}%", e.lang, percent(lang_metric(e, unit), total)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn format_lang_textblock(entries: &[LangEntry], unit: LangUnit) -> Vec<String> {
    let total = lang_total(entries, unit);
    if total == 0 {
        return Vec::new();
    }
    entries
        .iter()
        .map(|e| format_lang_line(e, total, unit))
        .collect()
}

/// Markdown emits a real bullet list — fixed-width column alignment doesn't survive
/// markdown rendering (whitespace is collapsed, soft line breaks merge lines), so each
/// language is its own list item with the dominant entry's name bolded.
fn format_lang_markdown(entries: &[LangEntry], unit: LangUnit) -> String {
    let total = lang_total(entries, unit);
    if total == 0 {
        return String::new();
    }
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| format_lang_markdown_row(e, total, unit, i == 0))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_lang_markdown_row(e: &LangEntry, total: u64, unit: LangUnit, dominant: bool) -> String {
    let pct = percent(lang_metric(e, unit), total);
    let metric = lang_metric(e, unit);
    let lang = if dominant {
        format!("**{}**", e.lang)
    } else {
        e.lang.clone()
    };
    format!("- {lang} — {pct}% · {metric} {}", unit.label())
}

fn format_lang_line(e: &LangEntry, total: u64, unit: LangUnit) -> String {
    let metric = lang_metric(e, unit);
    let pct = percent(metric, total);
    format!("{:<14} {pct:>3}%  {metric:>6} {}", e.lang, unit.label())
}

fn format_lang_entries(entries: &[LangEntry], unit: LangUnit) -> Vec<Entry> {
    let total = lang_total(entries, unit);
    entries
        .iter()
        .map(|e| {
            let metric = lang_metric(e, unit);
            let value = if total == 0 {
                format!("{metric} {}", unit.label())
            } else {
                format!("{}% ({metric} {})", percent(metric, total), unit.label())
            };
            Entry {
                key: e.lang.clone(),
                value: Some(value),
                status: None,
            }
        })
        .collect()
}

fn format_lang_ratio(entries: &[LangEntry], unit: LangUnit) -> Body {
    let total = lang_total(entries, unit);
    let (value, label, denominator) = match entries.first() {
        Some(top) if total > 0 => (
            lang_metric(top, unit) as f64 / total as f64,
            Some(top.lang.clone()),
            Some(total),
        ),
        _ => (0.0, None, None),
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator,
    })
}

fn percent(part: u64, total: u64) -> u64 {
    if total == 0 {
        return 0;
    }
    ((part as f64 / total as f64) * 100.0).round() as u64
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

    fn entry(lang: &str, lines: u64, files: u64) -> LangEntry {
        LangEntry {
            lang: lang.into(),
            lines,
            files,
        }
    }

    #[test]
    fn detect_language_resolves_common_extensions() {
        assert_eq!(detect_language("src/main.rs"), Some("Rust"));
        assert_eq!(detect_language("a/b.tsx"), Some("TypeScript"));
        assert_eq!(detect_language("script.PY"), Some("Python"));
        assert_eq!(detect_language("README.md"), Some("Markdown"));
    }

    #[test]
    fn detect_language_resolves_filenames_without_extension() {
        assert_eq!(detect_language("Dockerfile"), Some("Dockerfile"));
        assert_eq!(detect_language("docker/Dockerfile"), Some("Dockerfile"));
        assert_eq!(detect_language("Makefile"), Some("Makefile"));
        assert_eq!(detect_language("CMakeLists.txt"), Some("CMake"));
    }

    #[test]
    fn detect_language_returns_none_for_unknown_or_dotfile() {
        assert_eq!(detect_language("README"), None);
        assert_eq!(detect_language("LICENSE"), None);
        assert_eq!(detect_language(".gitignore"), None);
        assert_eq!(detect_language("file.unknownext"), None);
    }

    #[test]
    fn count_lines_handles_trailing_newline_or_not() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a"), 1);
        assert_eq!(count_lines(b"a\n"), 1);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
    }

    #[test]
    fn scans_languages_in_repo_grouped_by_extension() {
        // `commit_touching` appends `{msg}\n`, so each single-line `msg` ends up as 1 line.
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/a.rs", "fn a");
        commit_touching(&repo, "src/b.rs", "fn b");
        commit_touching(&repo, "web/x.ts", "let x");
        let scan = scan_languages_repo(&repo, LangUnit::Lines).unwrap();
        let labels: Vec<_> = scan.entries.iter().map(|e| e.lang.clone()).collect();
        assert_eq!(labels, vec!["Rust", "TypeScript"]);
        assert_eq!(scan.entries[0].lines, 2);
        assert_eq!(scan.entries[0].files, 2);
        assert_eq!(scan.entries[1].lines, 1);
        assert_eq!(scan.entries[1].files, 1);
    }

    #[test]
    fn scans_languages_skips_excluded_dirs() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn x() {}\n");
        commit_touching(&repo, "node_modules/dep.js", "x;\nx;\nx;\n");
        commit_touching(&repo, "vendor/lib.go", "package x\n");
        let scan = scan_languages_repo(&repo, LangUnit::Lines).unwrap();
        let labels: Vec<_> = scan.entries.iter().map(|e| e.lang.clone()).collect();
        assert_eq!(labels, vec!["Rust"]);
    }

    #[test]
    fn scans_languages_skips_unknown_and_binary() {
        let (tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn x");
        // unknown extension — no language match
        commit_touching(&repo, "data.unknownext", "1");
        // binary `.rs` — null bytes near the head force the binary filter.
        std::fs::write(tmp.path().join("blob.rs"), b"\x00\x01\x02fn x() {}\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "blob.rs"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-q", "-m", "bin"])
            .current_dir(tmp.path())
            .status()
            .unwrap();
        let scan = scan_languages_repo(&repo, LangUnit::Lines).unwrap();
        // Only `src/main.rs` (1 line) is counted; unknown extension and binary `.rs` are skipped.
        let rust = scan.entries.iter().find(|e| e.lang == "Rust").unwrap();
        assert_eq!(rust.files, 1);
        assert_eq!(rust.lines, 1);
        assert!(scan.entries.iter().all(|e| e.lang == "Rust"));
    }

    #[test]
    fn apply_lang_limit_collapses_tail_into_other() {
        let entries = vec![
            entry("Rust", 100, 5),
            entry("TS", 50, 4),
            entry("Python", 20, 2),
            entry("Go", 10, 1),
        ];
        let limited = apply_lang_limit(entries.clone(), 2, true);
        assert_eq!(limited.len(), 3);
        assert_eq!(limited[2].lang, "Other");
        assert_eq!(limited[2].lines, 30);
        assert_eq!(limited[2].files, 3);
    }

    #[test]
    fn apply_lang_limit_can_drop_tail() {
        let entries = vec![
            entry("Rust", 100, 5),
            entry("TS", 50, 4),
            entry("Py", 20, 2),
        ];
        let limited = apply_lang_limit(entries, 2, false);
        let labels: Vec<_> = limited.iter().map(|e| e.lang.clone()).collect();
        assert_eq!(labels, vec!["Rust", "TS"]);
    }

    #[test]
    fn apply_lang_limit_passes_through_when_under_limit() {
        let entries = vec![entry("Rust", 100, 5), entry("TS", 50, 4)];
        let limited = apply_lang_limit(entries.clone(), 10, true);
        assert_eq!(limited, entries);
    }

    #[test]
    fn render_text_summarises_top_three_with_percent() {
        let entries = vec![
            entry("Rust", 67, 1),
            entry("TS", 22, 1),
            entry("Py", 11, 1),
            entry("Go", 0, 1),
        ];
        let body = render_lang_body(entries, Shape::Text, LangUnit::Lines);
        match body {
            Body::Text(d) => assert_eq!(d.value, "Rust 67% · TS 22% · Py 11%"),
            _ => panic!(),
        }
    }

    #[test]
    fn render_text_is_empty_when_total_zero() {
        let body = render_lang_body(vec![], Shape::Text, LangUnit::Lines);
        match body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn render_bars_picks_unit() {
        let entries = vec![entry("Rust", 100, 5), entry("TS", 50, 9)];
        let by_lines = render_lang_body(entries.clone(), Shape::Bars, LangUnit::Lines);
        let by_files = render_lang_body(entries, Shape::Bars, LangUnit::Files);
        match by_lines {
            Body::Bars(d) => {
                assert_eq!(d.bars[0].label, "Rust");
                assert_eq!(d.bars[0].value, 100);
            }
            _ => panic!(),
        }
        match by_files {
            // Bars renders entries in the same order they came in (sort happens earlier in scan);
            // here we just confirm metric switches to file count.
            Body::Bars(d) => {
                assert_eq!(d.bars[0].value, 5);
                assert_eq!(d.bars[1].value, 9);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_ratio_exposes_dominance() {
        let entries = vec![entry("Rust", 67, 1), entry("TS", 33, 1)];
        let body = render_lang_body(entries, Shape::Ratio, LangUnit::Lines);
        match body {
            Body::Ratio(d) => {
                assert!((d.value - 0.67).abs() < 1e-9);
                assert_eq!(d.label, Some("Rust".into()));
                assert_eq!(d.denominator, Some(100));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_ratio_is_zero_when_empty() {
        let body = render_lang_body(vec![], Shape::Ratio, LangUnit::Lines);
        match body {
            Body::Ratio(d) => {
                assert_eq!(d.value, 0.0);
                assert!(d.label.is_none());
                assert!(d.denominator.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_badge_uses_dominant_lang() {
        let entries = vec![entry("Rust", 67, 1), entry("TS", 33, 1)];
        let body = render_lang_body(entries, Shape::Badge, LangUnit::Lines);
        match body {
            Body::Badge(d) => {
                assert_eq!(d.label, "Rust");
                assert_eq!(d.status, Status::Ok);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_markdown_emits_bullet_list_with_dominant_bolded() {
        let entries = vec![entry("Rust", 100, 1), entry("TS", 50, 1)];
        let body = render_lang_body(entries, Shape::MarkdownTextBlock, LangUnit::Lines);
        match body {
            Body::MarkdownTextBlock(d) => {
                let first = d.value.lines().next().unwrap();
                // Bullet + bold-name + percent + metric.
                assert_eq!(first, "- **Rust** — 67% · 100 lines");
                // Non-dominant rows: bullet + plain name.
                assert!(d.value.contains("\n- TS — 33% · 50 lines"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_entries_formats_pct_and_metric() {
        let entries = vec![entry("Rust", 80, 2), entry("TS", 20, 1)];
        let body = render_lang_body(entries, Shape::Entries, LangUnit::Lines);
        match body {
            Body::Entries(d) => {
                assert_eq!(d.items[0].key, "Rust");
                assert_eq!(d.items[0].value.as_deref(), Some("80% (80 lines)"));
                assert_eq!(d.items[1].value.as_deref(), Some("20% (20 lines)"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn render_number_series_returns_metric_per_lang() {
        let entries = vec![entry("Rust", 100, 5), entry("TS", 50, 9)];
        let body = render_lang_body(entries, Shape::NumberSeries, LangUnit::Files);
        match body {
            Body::NumberSeries(d) => assert_eq!(d.values, vec![5, 9]),
            _ => panic!(),
        }
    }

    #[test]
    fn percent_rounds_to_nearest_integer() {
        assert_eq!(percent(0, 100), 0);
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(33, 100), 33);
        assert_eq!(percent(2, 3), 67);
    }

    #[test]
    fn lang_unit_parses_from_toml() {
        let opts: LangOptions = toml::from_str(r#"unit = "files""#).unwrap();
        assert_eq!(opts.unit, Some(LangUnit::Files));
        let opts: LangOptions = toml::from_str(r#"unit = "lines""#).unwrap();
        assert_eq!(opts.unit, Some(LangUnit::Lines));
    }

    #[test]
    fn lang_options_reject_unknown_keys() {
        let res: Result<LangOptions, _> = toml::from_str(r#"foo = 1"#);
        assert!(res.is_err());
    }

    #[test]
    fn code_languages_cache_key_changes_with_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let key_default = CodeLanguages.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"unit = "files""#).unwrap());
        let key_files = CodeLanguages.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"unit = "lines""#).unwrap());
        let key_lines = CodeLanguages.cache_key(&ctx);
        assert_ne!(key_default, key_files);
        assert_ne!(key_files, key_lines);
    }
}
