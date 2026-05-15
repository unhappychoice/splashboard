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
    fn refresh_interval(&self) -> u64 {
        60 * 10
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

    fn ctx(shape: Option<Shape>, raw: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "widget".into(),
            shape,
            options: raw.map(|s| toml::from_str(s).unwrap()),
            ..Default::default()
        }
    }

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

    /// Two languages with distinct comment ratios drive `scan_repo`'s production sort_by
    /// closure (the dedicated `sort_order_picks_higher_ratio_first` test below uses an inline
    /// sort and doesn't touch the closure inside `scan_repo`). A Rust file has one comment
    /// against one code line (50%), while a Python file has only code (0%) — partial_cmp
    /// returns Some(non-Equal) and the high-ratio entry must come first.
    #[test]
    fn scan_repo_sorts_languages_by_descending_comment_ratio() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn main() {}\n// note\n");
        commit_touching(&repo, "script.py", "x = 1\ny = 2\nz = 3\n");
        let t = scan_repo(&repo).unwrap();
        let names: Vec<_> = t.by_language.iter().map(|(n, _)| n.as_str()).collect();
        let rust = names.iter().position(|n| *n == "Rust").expect("Rust entry");
        let python = names
            .iter()
            .position(|n| *n == "Python")
            .expect("Python entry");
        assert!(
            rust < python,
            "higher comment ratio (Rust 50%) must outrank lower (Python 0%); got {names:?}",
        );
    }

    /// Two languages tied on comment ratio (both 0%) fall through to the secondary `then_with`
    /// arm of `scan_repo`'s production sort closure, which orders by name ascending.
    #[test]
    fn scan_repo_sort_breaks_ratio_ties_alphabetically_by_language_name() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn main() {}\n");
        commit_touching(&repo, "script.py", "x = 1\n");
        let t = scan_repo(&repo).unwrap();
        let names: Vec<_> = t.by_language.iter().map(|(n, _)| n.as_str()).collect();
        let rust = names.iter().position(|n| *n == "Rust").expect("Rust entry");
        let python = names
            .iter()
            .position(|n| *n == "Python")
            .expect("Python entry");
        assert!(
            python < rust,
            "ratio tie must order by name ascending — Python before Rust; got {names:?}",
        );
    }

    /// Direct call to `ratio_of` with a zero-denominator stat exercises the `if denom == 0`
    /// arm. Production callers can't reach it (the `code + comments > 0` filter in `scan_repo`
    /// drops empty entries before they make it into `by_language`), so the only way to pin this
    /// branch is to feed `ratio_of` an empty `LangStat` directly.
    #[test]
    fn ratio_of_returns_zero_for_empty_lang_stat() {
        assert_eq!(ratio_of(&LangStat::default()), 0.0);
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
        assert!(matches!(
            body,
            Body::Text(d)
                if d.value.contains("20.0%")
                    && d.value.contains("250")
                    && d.value.contains("1,250"),
        ));
    }

    #[test]
    fn text_is_empty_when_no_lines() {
        let body = render_text(&Totals::default());
        assert!(matches!(body, Body::Text(d) if d.value.is_empty()));
    }

    #[test]
    fn ratio_uses_total_comments_over_total_code_plus_comments() {
        let body = render_ratio(&totals(&[("Rust", 800, 200)]));
        assert!(matches!(
            body,
            Body::Ratio(d)
                if (d.value - 0.2).abs() < 1e-9
                    && d.denominator == Some(1000)
                    && d.label.as_deref() == Some("comments"),
        ));
    }

    #[test]
    fn ratio_handles_empty() {
        let body = render_ratio(&Totals::default());
        assert!(matches!(
            body,
            Body::Ratio(d)
                if d.value == 0.0 && d.denominator.is_none() && d.label.is_none(),
        ));
    }

    /// `render_body(Shape::Ratio, ...)` delegates to `render_ratio`; covers the otherwise-skipped
    /// `Shape::Ratio` arm of the dispatch table (the dedicated `render_ratio` tests above bypass it).
    #[test]
    fn render_body_ratio_arm_delegates_to_render_ratio() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::Ratio,
            10,
            Unit::Percent,
        );
        assert!(matches!(
            body,
            Body::Ratio(d)
                if (d.value - 0.2).abs() < 1e-9
                    && d.denominator == Some(1000)
                    && d.label.as_deref() == Some("comments"),
        ));
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
        assert!(matches!(
            body,
            Body::TextBlock(d)
                if d.lines[0].contains("Markdown")
                    && d.lines[0].contains("50.0%")
                    && d.lines[1].contains("Rust")
                    && d.lines[1].contains("20.0%"),
        ));
    }

    #[test]
    fn markdown_text_block_emits_bold_list() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::MarkdownTextBlock,
            10,
            Unit::Percent,
        );
        assert!(matches!(
            body,
            Body::MarkdownTextBlock(d) if d.value == "- **Rust** 20.0%",
        ));
    }

    #[test]
    fn entries_default_to_percent() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::Entries,
            10,
            Unit::Percent,
        );
        assert!(matches!(
            body,
            Body::Entries(d) if d.items[0].value.as_deref() == Some("20.0%"),
        ));
    }

    #[test]
    fn entries_with_loc_unit_show_raw_comment_count() {
        let body = render_body(
            totals(&[("Rust", 800, 1234)]),
            Shape::Entries,
            10,
            Unit::Loc,
        );
        assert!(matches!(
            body,
            Body::Entries(d) if d.items[0].value.as_deref() == Some("1,234"),
        ));
    }

    #[test]
    fn bars_in_percent_mode_use_basis_points() {
        let body = render_body(
            totals(&[("Rust", 800, 200)]),
            Shape::Bars,
            10,
            Unit::Percent,
        );
        // 20.0% × 10 = 200 basis points.
        assert!(matches!(body, Body::Bars(d) if d.bars[0].value == 200));
    }

    #[test]
    fn bars_in_loc_mode_use_raw_comments() {
        let body = render_body(totals(&[("Rust", 800, 1234)]), Shape::Bars, 10, Unit::Loc);
        assert!(matches!(body, Body::Bars(d) if d.bars[0].value == 1234));
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
        assert!(matches!(
            body,
            Body::Badge(d) if d.status == Status::Ok && d.label == "empty",
        ));
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

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        let fetcher = CodeComments;
        let schema_names: Vec<_> = fetcher
            .option_schemas()
            .iter()
            .map(|schema| schema.name)
            .collect();
        assert_eq!(fetcher.name(), "code_comments");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.default_shape(), Shape::Text);
        assert!(
            fetcher
                .description()
                .contains("Comment-density per language")
        );
        assert_eq!(schema_names, vec!["limit", "unit"]);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert!(
            SHAPES
                .iter()
                .copied()
                .all(|shape| fetcher.sample_body(shape).is_some())
        );
        assert!(fetcher.sample_body(Shape::Image).is_none());
    }

    #[test]
    fn render_body_falls_back_to_text_for_unsupported_shapes() {
        let body = render_body(
            totals(&[("Rust", 800, 200), ("TOML", 90, 10)]),
            Shape::Image,
            1,
            Unit::Percent,
        );
        assert!(matches!(
            body,
            Body::Text(d) if d.value == "19.1% comments · 210 / 1,100 lines",
        ));
    }

    #[test]
    fn format_value_kloc_scales_thousands_and_millions() {
        assert_eq!(format_value(&lang_stat(0, 999), Unit::Kloc), "999");
        assert_eq!(format_value(&lang_stat(0, 1_234), Unit::Kloc), "1.2k");
        assert_eq!(format_value(&lang_stat(0, 1_234_567), Unit::Kloc), "1.2M");
    }

    #[test]
    fn parse_options_defaults_when_missing() {
        let opts = parse_options(None).unwrap();
        assert!(opts.limit.is_none());
        assert!(opts.unit.is_none());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options() {
        let err = CodeComments
            .fetch(&ctx(None, Some(r#"bogus = true"#)))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid options"));
    }

    #[test]
    fn fetch_scans_cwd_repo_for_multiple_shapes() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (_tmp, repo) = crate::fetcher::git::test_support::make_repo();
        crate::fetcher::git::test_support::commit_touching(
            &repo,
            "src/lib.rs",
            "// a code comment\nfn main() {}\n",
        );
        let workdir = repo.workdir().unwrap().to_path_buf();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workdir).unwrap();

        let text = rt.block_on(CodeComments.fetch(&ctx(Some(Shape::Text), None)));
        let badge = rt.block_on(CodeComments.fetch(&ctx(Some(Shape::Badge), Some(r#"limit = 1"#))));
        let entries = rt.block_on(CodeComments.fetch(&ctx(
            Some(Shape::Entries),
            Some(
                r#"
limit = 1
unit = "kloc"
"#,
            ),
        )));

        std::env::set_current_dir(prev_cwd).unwrap();

        assert!(matches!(
            text.unwrap().body,
            Body::Text(d) if d.value.contains("comments"),
        ));
        assert!(matches!(
            badge.unwrap().body,
            Body::Badge(d) if !d.label.is_empty(),
        ));
        assert!(matches!(
            entries.unwrap().body,
            Body::Entries(d)
                if d.items.len() == 1
                    && !d.items[0].key.is_empty()
                    && !d.items[0].value.as_deref().unwrap_or_default().is_empty(),
        ));
    }
}
