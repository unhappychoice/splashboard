use std::collections::HashMap;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{Bar, BarsData, Body, EntriesData, Entry, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::languages;
use super::scan::for_each_tracked_file;

const SHAPES: &[Shape] = &[Shape::Text, Shape::TextBlock, Shape::Entries, Shape::Bars];
const DEFAULT_LIMIT: usize = 10;
const OTHER_LABEL: &str = "Other";

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "limit",
    type_hint: "integer",
    required: false,
    default: Some("10"),
    description: "Cap on rendered languages (`TextBlock` / `Entries` / `Bars`). The `Text` summary always reports the full count.",
}];

/// Counts lines per language across tracked source files in the discovered git repo.
/// Languages are inferred from extension / bare-filename map; unknown files bucket into
/// `"Other"`. `Text` summarises `"N lines across M languages"`; `TextBlock` lists one
/// `"Language  count"` per row; `Entries` / `Bars` rank languages by line count. Walks the
/// `gix` index so untracked / `.gitignore`-d files and committed-vendored trees
/// (`node_modules/`, `vendor/`, `dist/`, `target/`, …) are skipped automatically.
pub struct CodeLoc;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Fetcher for CodeLoc {
    fn name(&self) -> &str {
        "code_loc"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Counts lines per language across tracked source files in the discovered git repo (extension-based classification, vendored / generated dirs skipped). `Text` summarises `\"N lines across M languages\"`; `TextBlock` / `Entries` / `Bars` rank languages by line count."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        // Mix `[widget.options]` into the key — `limit` changes what gets rendered, so two
        // widgets pointing at this fetcher with different options must not share a cache slot.
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
            Shape::Text => samples::text("12,345 lines across 6 languages"),
            Shape::TextBlock => samples::text_block(&[
                "Rust         8,234",
                "TypeScript   2,111",
                "Markdown       820",
                "TOML           560",
            ]),
            Shape::Entries => samples::entries(&[
                ("Rust", "8,234"),
                ("TypeScript", "2,111"),
                ("Markdown", "820"),
                ("TOML", "560"),
            ]),
            Shape::Bars => samples::bars(&[
                ("Rust", 8234),
                ("TypeScript", 2111),
                ("Markdown", 820),
                ("TOML", 560),
            ]),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let totals = scan_cwd()?;
        Ok(payload(render_body(
            totals,
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

#[derive(Debug, Default)]
struct Totals {
    /// Sorted by line count desc, then language name asc.
    by_language: Vec<(String, u64)>,
    total_lines: u64,
}

fn scan_cwd() -> Result<Totals, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo)
}

fn scan_repo(repo: &gix::Repository) -> Result<Totals, FetchError> {
    let mut counts: HashMap<&'static str, u64> = HashMap::new();
    let mut other_lines: u64 = 0;
    let mut total: u64 = 0;
    for_each_tracked_file(repo, |path, bytes| {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return;
        };
        let lines = text.lines().count() as u64;
        if lines == 0 {
            return;
        }
        total += lines;
        match languages::classify(path) {
            Some(lang) => *counts.entry(lang).or_insert(0) += lines,
            None => other_lines += lines,
        }
    })?;
    let mut by_language: Vec<(String, u64)> = counts
        .into_iter()
        .map(|(lang, n)| (lang.to_string(), n))
        .collect();
    if other_lines > 0 {
        by_language.push((OTHER_LABEL.to_string(), other_lines));
    }
    by_language.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Totals {
        by_language,
        total_lines: total,
    })
}

fn render_body(totals: Totals, shape: Shape, limit: usize) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: totals
                .by_language
                .into_iter()
                .take(limit)
                .map(|(lang, n)| format!("{:<14} {}", lang, format_with_commas(n)))
                .collect(),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: totals
                .by_language
                .into_iter()
                .take(limit)
                .map(|(lang, n)| Entry {
                    key: lang,
                    value: Some(format_with_commas(n)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: totals
                .by_language
                .into_iter()
                .take(limit)
                .map(|(lang, n)| Bar {
                    label: lang,
                    value: n,
                })
                .collect(),
        }),
        _ => text_summary(totals.total_lines, totals.by_language.len()),
    }
}

fn text_summary(total: u64, langs: usize) -> Body {
    let value = if total == 0 {
        String::new()
    } else {
        format!(
            "{} line{} across {langs} language{}",
            format_with_commas(total),
            if total == 1 { "" } else { "s" },
            if langs == 1 { "" } else { "s" },
        )
    };
    Body::Text(TextData { value })
}

fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::super::git::test_support::{commit_touching, make_repo};
    use super::*;

    fn lang_map(totals: &Totals) -> HashMap<String, u64> {
        totals.by_language.iter().cloned().collect()
    }

    #[test]
    fn empty_repo_returns_empty_totals() {
        let (_tmp, repo) = make_repo();
        let totals = scan_repo(&repo).unwrap();
        assert_eq!(totals.total_lines, 0);
        assert!(totals.by_language.is_empty());
    }

    #[test]
    fn counts_lines_per_language() {
        let (_tmp, repo) = make_repo();
        // commit_touching writes "<msg>\n" — content for src/main.rs becomes "fn main() {}\n// hi\n"
        // (`text.lines().count()` → 2). README.md's Markdown content becomes "# hi\n\ntext\n" → 3 lines.
        commit_touching(&repo, "src/main.rs", "fn main() {}\n// hi");
        commit_touching(&repo, "doc.md", "# hi\n\ntext");
        let totals = scan_repo(&repo).unwrap();
        let langs = lang_map(&totals);
        assert_eq!(langs.get("Rust"), Some(&2));
        assert!(langs.get("Markdown").copied().unwrap_or(0) >= 3);
    }

    #[test]
    fn unknown_extensions_bucket_into_other() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "weird.xyz", "a\nb\nc");
        let totals = scan_repo(&repo).unwrap();
        let langs = lang_map(&totals);
        // "a\nb\nc\n" → 3 lines under "Other"
        assert_eq!(langs.get("Other"), Some(&3));
    }

    #[test]
    fn vendored_dirs_are_excluded() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn a() {}");
        commit_touching(&repo, "node_modules/x.js", "var a = 1;");
        commit_touching(&repo, "vendor/y.go", "package x");
        let totals = scan_repo(&repo).unwrap();
        let langs = lang_map(&totals);
        assert!(langs.contains_key("Rust"));
        assert!(!langs.contains_key("JavaScript"));
        assert!(!langs.contains_key("Go"));
    }

    #[test]
    fn lockfiles_are_excluded() {
        // Cargo.lock alone is ~6k lines on a typical Rust repo — without this exclusion the
        // "Other" bucket dominates every real source language.
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn main() {}");
        commit_touching(&repo, "Cargo.lock", "noise\nnoise\nnoise");
        commit_touching(&repo, "frontend/package-lock.json", "{}\n{}");
        let totals = scan_repo(&repo).unwrap();
        let langs = lang_map(&totals);
        assert!(langs.contains_key("Rust"));
        assert!(!langs.contains_key("Other"));
        assert!(!langs.contains_key("JSON"));
    }

    #[test]
    fn ranks_languages_by_line_count_descending() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a.rs", "1\n2\n3\n4\n5");
        commit_touching(&repo, "b.py", "1\n2");
        let totals = scan_repo(&repo).unwrap();
        let labels: Vec<_> = totals.by_language.iter().map(|(l, _)| l.clone()).collect();
        assert_eq!(labels.first().map(String::as_str), Some("Rust"));
        assert_eq!(labels.get(1).map(String::as_str), Some("Python"));
    }

    #[test]
    fn text_summary_reports_total_and_language_count() {
        let body = render_body(
            Totals {
                by_language: vec![("Rust".into(), 1234), ("Python".into(), 567)],
                total_lines: 1801,
            },
            Shape::Text,
            10,
        );
        match body {
            Body::Text(d) => assert_eq!(d.value, "1,801 lines across 2 languages"),
            _ => panic!(),
        }
    }

    #[test]
    fn text_summary_handles_singular_grammar() {
        let body = render_body(
            Totals {
                by_language: vec![("Rust".into(), 1)],
                total_lines: 1,
            },
            Shape::Text,
            10,
        );
        match body {
            Body::Text(d) => assert_eq!(d.value, "1 line across 1 language"),
            _ => panic!(),
        }
    }

    #[test]
    fn text_summary_is_empty_when_no_lines() {
        let body = render_body(Totals::default(), Shape::Text, 10);
        match body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn entries_carry_language_and_count() {
        let body = render_body(
            Totals {
                by_language: vec![("Rust".into(), 1234)],
                total_lines: 1234,
            },
            Shape::Entries,
            10,
        );
        match body {
            Body::Entries(d) => {
                assert_eq!(d.items[0].key, "Rust");
                assert_eq!(d.items[0].value.as_deref(), Some("1,234"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn bars_carry_language_and_count() {
        let body = render_body(
            Totals {
                by_language: vec![("Rust".into(), 1234), ("Python".into(), 56)],
                total_lines: 1290,
            },
            Shape::Bars,
            10,
        );
        match body {
            Body::Bars(d) => {
                assert_eq!(d.bars.len(), 2);
                assert_eq!(d.bars[0].label, "Rust");
                assert_eq!(d.bars[0].value, 1234);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_one_row_per_language_with_commas() {
        let body = render_body(
            Totals {
                by_language: vec![("Rust".into(), 1234), ("Python".into(), 56)],
                total_lines: 1290,
            },
            Shape::TextBlock,
            10,
        );
        match body {
            Body::TextBlock(d) => {
                assert_eq!(d.lines.len(), 2);
                assert!(d.lines[0].contains("Rust"));
                assert!(d.lines[0].contains("1,234"));
                assert!(d.lines[1].contains("Python"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn limit_caps_rendered_rows() {
        let totals = Totals {
            by_language: (0..5).map(|i| (format!("L{i}"), 1)).collect(),
            total_lines: 5,
        };
        let body = render_body(totals, Shape::Entries, 2);
        match body {
            Body::Entries(d) => assert_eq!(d.items.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn format_with_commas_inserts_separators() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(123), "123");
        assert_eq!(format_with_commas(1_234), "1,234");
        assert_eq!(format_with_commas(1_234_567), "1,234,567");
    }

    #[test]
    fn cache_key_changes_with_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let k0 = CodeLoc.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"limit = 5"#).unwrap());
        let k1 = CodeLoc.cache_key(&ctx);
        assert_ne!(k0, k1);
    }

    #[test]
    fn cache_key_is_stable_for_equivalent_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        ctx.options = Some(toml::from_str(r#"limit = 7"#).unwrap());
        let a = CodeLoc.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"limit = 7"#).unwrap());
        let b = CodeLoc.cache_key(&ctx);
        assert_eq!(a, b);
    }

    #[test]
    fn parse_options_rejects_unknown_keys() {
        let bad: toml::Value = toml::from_str(r#"unknown = 1"#).unwrap();
        assert!(parse_options(Some(&bad)).is_err());
    }
}
