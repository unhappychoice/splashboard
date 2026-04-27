//! `code_comments` — comment density per language across tracked source files. Pairs with
//! `code_loc` (which counts physical lines) to surface the documentation posture of the
//! codebase: "Rust 18% comments / TypeScript 4%" tells you which language family carries
//! the team's writing.

use std::collections::HashMap;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, RatioData,
    Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::for_each_tokei_stat;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::Ratio,
    Shape::Badge,
];
const DEFAULT_LIMIT: usize = 10;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("10"),
        description: "Cap on rendered languages (`TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars`). The `Text` summary always reports the whole-repo ratio.",
    },
    OptionSchema {
        name: "unit",
        type_hint: "`percent` (alias `%`) | `loc` | `kloc`",
        required: false,
        default: Some("percent"),
        description: "Display format for per-language values: `percent` (`18.3%` of that language's `code+comments`), `loc` (raw comment lines, `1,234`), or `kloc` (`1.2k`). `Ratio` always emits the whole-repo comment share regardless of this option; `Badge` always reports tier + percent.",
    },
];

pub struct CodeComments;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    unit: Option<Unit>,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Unit {
    #[default]
    #[serde(alias = "%")]
    Percent,
    Loc,
    Kloc,
}

#[async_trait]
impl Fetcher for CodeComments {
    fn name(&self) -> &str {
        "code_comments"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Comment-density per language across tracked source files in the discovered git repo. tokei parses each file into `code` / `comments` / `blanks`; this fetcher surfaces the comment share. `Text` headlines the whole-repo ratio; `TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` rank per-language values (default `percent`, override with `unit = loc | kloc`); `Ratio` exposes the whole-repo share for gauges; `Badge` tiers documentation posture (`undocumented` / `light` / `balanced` / `documented` / `verbose`)."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
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
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("18.3% comments · 3,456 / 18,892 lines"),
            Shape::TextBlock => samples::text_block(&[
                "Rust         24.1%",
                "Markdown     58.0%",
                "TypeScript    9.4%",
                "TOML          4.2%",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **Rust** 24.1%\n- **Markdown** 58.0%\n- **TypeScript** 9.4%\n- **TOML** 4.2%",
            ),
            Shape::Entries => samples::entries(&[
                ("Rust", "24.1%"),
                ("Markdown", "58.0%"),
                ("TypeScript", "9.4%"),
                ("TOML", "4.2%"),
            ]),
            Shape::Bars => samples::bars(&[
                ("Rust", 1900),
                ("Markdown", 580),
                ("TypeScript", 230),
                ("TOML", 32),
            ]),
            Shape::Ratio => samples::ratio(0.183, "comments"),
            Shape::Badge => samples::badge(Status::Ok, "balanced · 18%"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let unit = opts.unit.unwrap_or_default();
        let totals = scan_cwd()?;
        Ok(payload(render_body(
            totals,
            ctx.shape.unwrap_or(Shape::Text),
            limit,
            unit,
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

#[derive(Debug, Default, Clone)]
struct LangStat {
    code: u64,
    comments: u64,
}

#[derive(Debug, Default)]
struct Totals {
    /// Sorted by comment ratio desc (then by name asc), languages with zero `code+comments` dropped.
    by_language: Vec<(String, LangStat)>,
    total_code: u64,
    total_comments: u64,
}

fn scan_cwd() -> Result<Totals, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo)
}

fn scan_repo(repo: &gix::Repository) -> Result<Totals, FetchError> {
    let mut by: HashMap<&'static str, LangStat> = HashMap::new();
    let mut total_code = 0u64;
    let mut total_comments = 0u64;
    for_each_tokei_stat(repo, |_path, name, stats| {
        // Prose languages (Markdown / MDX / Plain Text / AsciiDoc) report every line as
        // `comments` and zero `code`, which makes their comment-density ratio collapse to
        // 100% and dominates the per-language ranking. They aren't documenting code, so
        // they don't belong in a code-comment-density metric. Filtering on `code > 0`
        // also drops empty / header-only files that would distort small-language ratios.
        if stats.code == 0 {
            return;
        }
        let entry = by.entry(name).or_default();
        entry.code += stats.code as u64;
        entry.comments += stats.comments as u64;
        total_code += stats.code as u64;
        total_comments += stats.comments as u64;
    })?;
    let mut by_language: Vec<(String, LangStat)> = by
        .into_iter()
        .filter(|(_, s)| s.code + s.comments > 0)
        .map(|(name, stat)| (name.to_string(), stat))
        .collect();
    by_language.sort_by(|a, b| {
        ratio_of(&b.1)
            .partial_cmp(&ratio_of(&a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    Ok(Totals {
        by_language,
        total_code,
        total_comments,
    })
}

fn ratio_of(stat: &LangStat) -> f64 {
    let denom = stat.code + stat.comments;
    if denom == 0 {
        0.0
    } else {
        stat.comments as f64 / denom as f64
    }
}

fn render_body(totals: Totals, shape: Shape, limit: usize, unit: Unit) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| format!("{:<12} {}", name, format_value(stat, unit)))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| format!("- **{name}** {}", format_value(stat, unit)))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| Entry {
                    key: name.clone(),
                    value: Some(format_value(stat, unit)),
                    status: None,
                })
                .collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: totals
                .by_language
                .iter()
                .take(limit)
                .map(|(name, stat)| Bar {
                    label: name.clone(),
                    value: bar_value(stat, unit),
                })
                .collect(),
        }),
        Shape::Ratio => render_ratio(&totals),
        Shape::Badge => render_badge(&totals),
        _ => render_text(&totals),
    }
}

fn render_text(totals: &Totals) -> Body {
    let denom = totals.total_code + totals.total_comments;
    if denom == 0 {
        return Body::Text(TextData {
            value: String::new(),
        });
    }
    let pct = (totals.total_comments as f64 / denom as f64) * 100.0;
    Body::Text(TextData {
        value: format!(
            "{:.1}% comments · {} / {} lines",
            pct,
            format_with_commas(totals.total_comments),
            format_with_commas(denom),
        ),
    })
}

fn render_ratio(totals: &Totals) -> Body {
    let denom = totals.total_code + totals.total_comments;
    let (value, denominator, label) = if denom == 0 {
        (0.0, None, None)
    } else {
        (
            (totals.total_comments as f64 / denom as f64).clamp(0.0, 1.0),
            Some(denom),
            Some("comments".into()),
        )
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator,
    })
}

fn render_badge(totals: &Totals) -> Body {
    let denom = totals.total_code + totals.total_comments;
    if denom == 0 {
        return Body::Badge(BadgeData {
            status: Status::Ok,
            label: "empty".into(),
        });
    }
    let pct = (totals.total_comments as f64 / denom as f64) * 100.0;
    let (tier, status) = tier_for(pct);
    Body::Badge(BadgeData {
        status,
        label: format!("{tier} · {pct:.0}%"),
    })
}

fn tier_for(pct: f64) -> (&'static str, Status) {
    if pct < 5.0 {
        ("undocumented", Status::Warn)
    } else if pct < 15.0 {
        ("light", Status::Ok)
    } else if pct < 30.0 {
        ("balanced", Status::Ok)
    } else if pct < 50.0 {
        ("documented", Status::Ok)
    } else {
        ("verbose", Status::Warn)
    }
}

fn format_value(stat: &LangStat, unit: Unit) -> String {
    match unit {
        Unit::Percent => format!("{:.1}%", ratio_of(stat) * 100.0),
        Unit::Loc => format_with_commas(stat.comments),
        Unit::Kloc => format_kloc(stat.comments),
    }
}

fn bar_value(stat: &LangStat, unit: Unit) -> u64 {
    match unit {
        // Percent shape on Bars: scale to basis points (×10) so renderers using integer
        // heights still differentiate "8.3%" from "8.7%".
        Unit::Percent => (ratio_of(stat) * 1000.0).round() as u64,
        Unit::Loc | Unit::Kloc => stat.comments,
    }
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

fn format_kloc(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::git::test_support::{commit_touching, make_repo};
    use super::*;

    fn lang_stat(code: u64, comments: u64) -> LangStat {
        LangStat { code, comments }
    }

    fn totals(items: &[(&str, u64, u64)]) -> Totals {
        let total_code = items.iter().map(|(_, c, _)| c).sum();
        let total_comments = items.iter().map(|(_, _, m)| m).sum();
        Totals {
            by_language: items
                .iter()
                .map(|(l, c, m)| ((*l).into(), lang_stat(*c, *m)))
                .collect(),
            total_code,
            total_comments,
        }
    }

    #[test]
    fn empty_repo_returns_empty_totals() {
        let (_tmp, repo) = make_repo();
        let t = scan_repo(&repo).unwrap();
        assert_eq!(t.total_code, 0);
        assert_eq!(t.total_comments, 0);
        assert!(t.by_language.is_empty());
    }

    #[test]
    fn rust_file_with_line_comment_separates_code_and_comments() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn main() {}\n// hello\n// world\n");
        let t = scan_repo(&repo).unwrap();
        assert!(t.total_code >= 1);
        assert!(t.total_comments >= 2);
    }

    #[test]
    fn prose_files_with_zero_code_lines_are_excluded() {
        // Markdown / Plain Text classify every line as `comments` per tokei → ratio
        // collapses to 100% and pollutes the ranking. The `code > 0` filter drops them so
        // the metric reflects code-comment density only.
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "README.md", "# title\n\nbody text\nmore prose\n");
        commit_touching(&repo, "src/main.rs", "fn main() {}\n// note\n");
        let t = scan_repo(&repo).unwrap();
        assert!(
            t.by_language.iter().all(|(n, _)| n != "Markdown"),
            "Markdown should be filtered out, got {:?}",
            t.by_language
        );
        assert!(t.by_language.iter().any(|(n, _)| n == "Rust"));
    }

    #[test]
    fn rust_file_is_partitioned_into_code_and_comment_lines() {
        let (_tmp, repo) = make_repo();
        commit_touching(
            &repo,
            "src/main.rs",
            "// header\nfn main() {}\n// trailing\n",
        );
        let t = scan_repo(&repo).unwrap();
        let rust = t
            .by_language
            .iter()
            .find(|(n, _)| n == "Rust")
            .expect("Rust entry");
        assert_eq!(rust.1.code, 1);
        assert_eq!(rust.1.comments, 2);
    }

    #[test]
    fn sort_order_picks_higher_ratio_first() {
        // Hand-built Totals — bypasses tokei's per-language quirks (Markdown / TOML
        // bucketing varies across versions). Tests the sort key only.
        let mut t = Totals {
            by_language: vec![
                ("LowRatio".into(), lang_stat(900, 100)),  // 10%
                ("HighRatio".into(), lang_stat(500, 500)), // 50%
            ],
            total_code: 1400,
            total_comments: 600,
        };
        t.by_language.sort_by(|a, b| {
            ratio_of(&b.1)
                .partial_cmp(&ratio_of(&a.1))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        assert_eq!(t.by_language[0].0, "HighRatio");
        assert_eq!(t.by_language[1].0, "LowRatio");
    }

    #[test]
    fn text_reports_whole_repo_ratio_and_counts() {
        let body = render_text(&totals(&[("Rust", 800, 200), ("Markdown", 200, 50)]));
        match body {
            Body::Text(d) => {
                assert!(d.value.contains("20.0%"));
                assert!(d.value.contains("250"));
                assert!(d.value.contains("1,250"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_is_empty_when_no_lines() {
        let body = render_text(&Totals::default());
        match body {
            Body::Text(d) => assert!(d.value.is_empty()),
            _ => panic!(),
        }
    }

    #[test]
    fn ratio_uses_total_comments_over_total_code_plus_comments() {
        let body = render_ratio(&totals(&[("Rust", 800, 200)]));
        match body {
            Body::Ratio(d) => {
                assert!((d.value - 0.2).abs() < 1e-9);
                assert_eq!(d.denominator, Some(1000));
                assert_eq!(d.label.as_deref(), Some("comments"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn ratio_handles_empty() {
        let body = render_ratio(&Totals::default());
        match body {
            Body::Ratio(d) => {
                assert_eq!(d.value, 0.0);
                assert!(d.denominator.is_none());
                assert!(d.label.is_none());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn text_block_lists_per_language_values() {
        // `totals()` preserves input order — feed pre-sorted by ratio desc to match what
        // `scan_repo` would produce.
        let body = render_body(
            totals(&[("Markdown", 100, 100), ("Rust", 800, 200)]),
            Shape::TextBlock,
            10,
            Unit::Percent,
        );
        match body {
            Body::TextBlock(d) => {
                assert!(d.lines[0].contains("Markdown"));
                assert!(d.lines[0].contains("50.0%"));
                assert!(d.lines[1].contains("Rust"));
                assert!(d.lines[1].contains("20.0%"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn markdown_text_block_emits_bold_list() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::MarkdownTextBlock,
            10,
            Unit::Percent,
        );
        match body {
            Body::MarkdownTextBlock(d) => assert_eq!(d.value, "- **Rust** 20.0%"),
            _ => panic!(),
        }
    }

    #[test]
    fn entries_default_to_percent() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::Entries,
            10,
            Unit::Percent,
        );
        match body {
            Body::Entries(d) => assert_eq!(d.items[0].value.as_deref(), Some("20.0%")),
            _ => panic!(),
        }
    }

    #[test]
    fn entries_with_loc_unit_show_raw_comment_count() {
        let body = render_body(
            totals(&[("Rust", 800, 1234)]),
            Shape::Entries,
            10,
            Unit::Loc,
        );
        match body {
            Body::Entries(d) => assert_eq!(d.items[0].value.as_deref(), Some("1,234")),
            _ => panic!(),
        }
    }

    #[test]
    fn bars_in_percent_mode_use_basis_points() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::Bars,
            10,
            Unit::Percent,
        );
        match body {
            Body::Bars(d) => assert_eq!(d.bars[0].value, 200), // 20.0% × 10
            _ => panic!(),
        }
    }

    #[test]
    fn bars_in_loc_mode_use_raw_comments() {
        let body = render_body(totals(&[("Rust", 800, 1234)]), Shape::Bars, 10, Unit::Loc);
        match body {
            Body::Bars(d) => assert_eq!(d.bars[0].value, 1234),
            _ => panic!(),
        }
    }

    #[test]
    fn badge_tiers_by_pct() {
        assert_eq!(tier_for(2.0), ("undocumented", Status::Warn));
        assert_eq!(tier_for(10.0), ("light", Status::Ok));
        assert_eq!(tier_for(20.0), ("balanced", Status::Ok));
        assert_eq!(tier_for(40.0), ("documented", Status::Ok));
        assert_eq!(tier_for(60.0), ("verbose", Status::Warn));
    }

    #[test]
    fn badge_handles_empty() {
        let body = render_body(Totals::default(), Shape::Badge, 10, Unit::Percent);
        match body {
            Body::Badge(d) => {
                assert_eq!(d.status, Status::Ok);
                assert_eq!(d.label, "empty");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_options_rejects_unknown_keys() {
        let bad: toml::Value = toml::from_str(r#"unknown = 1"#).unwrap();
        assert!(parse_options(Some(&bad)).is_err());
    }

    #[test]
    fn parse_options_accepts_unit_aliases() {
        let pct: toml::Value = toml::from_str(r#"unit = "%""#).unwrap();
        assert_eq!(parse_options(Some(&pct)).unwrap().unit, Some(Unit::Percent));
        let loc: toml::Value = toml::from_str(r#"unit = "loc""#).unwrap();
        assert_eq!(parse_options(Some(&loc)).unwrap().unit, Some(Unit::Loc));
    }

    #[test]
    fn cache_key_changes_with_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let a = CodeComments.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"unit = "loc""#).unwrap());
        let b = CodeComments.cache_key(&ctx);
        assert_ne!(a, b);
    }
}
