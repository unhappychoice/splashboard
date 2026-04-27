//! `github_languages` — language byte-count breakdown for the current repo, via
//! `/repos/{o}/{n}/languages`. The response is a JSON object `{ "Rust": N, "TOML": M }`; we
//! sort by size descending and emit `Bars` (for chart_pie / chart_bar) or `Entries` with
//! percent values (for grid_table inline / rows).

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, TextBlockData,
    TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::client::rest_get;
use super::common::{RepoSlug, cache_key, parse_options, payload, resolve_repo};

const SHAPES: &[Shape] = &[
    Shape::Bars,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Text,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "repo",
        type_hint: "\"owner/name\"",
        required: false,
        default: Some("git remote of cwd"),
        description: "Repository to query. Falls back to the current directory's github remote.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer (1..=20)",
        required: false,
        default: Some("6"),
        description: "Maximum number of languages to surface. Smaller slices collapse into the tail.",
    },
];

const DEFAULT_LIMIT: usize = 6;

pub struct GithubLanguages;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[async_trait]
impl Fetcher for GithubLanguages {
    fn name(&self) -> &str {
        "github_languages"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Language byte-count breakdown for a repo, sorted by size. `Bars` / `Entries` / `TextBlock` / `MarkdownTextBlock` carry the full ranking with percent values; `Text` collapses to a `\"Rust 87% · TOML 8% · …\"` headline. Languages beyond `limit` collapse into a single `other` bucket so totals stay honest."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Bars
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = repo_for_key(ctx);
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Bars => samples::bars(&[("Rust", 87_000), ("TOML", 8_000), ("Shell", 5_000)]),
            Shape::Entries => samples::entries(&[("Rust", "87%"), ("TOML", "8%"), ("Shell", "5%")]),
            Shape::TextBlock => samples::text_block(&["Rust  87.0%", "TOML  8.0%", "Shell  5.0%"]),
            Shape::MarkdownTextBlock => {
                samples::markdown("- **Rust** — 87.0%\n- **TOML** — 8.0%\n- **Shell** — 5.0%")
            }
            Shape::Text => samples::text("Rust 87% · TOML 8% · Shell 5%"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let slug = resolve_repo(opts.repo.as_deref())?;
        let limit = opts
            .limit
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .max(1);
        let raw: BTreeMap<String, u64> =
            rest_get(&format!("/repos/{}/{}/languages", slug.owner, slug.name)).await?;
        let body = build_body(raw, ctx.shape.unwrap_or(Shape::Bars), limit);
        Ok(payload(body))
    }
}

fn build_body(raw: BTreeMap<String, u64>, shape: Shape, limit: usize) -> Body {
    let sorted = top_n(raw, limit);
    match shape {
        Shape::Entries => Body::Entries(EntriesData {
            items: to_entries(&sorted),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: text_lines(&sorted),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_text(&sorted),
        }),
        Shape::Text => Body::Text(TextData {
            value: text_headline(&sorted),
        }),
        _ => Body::Bars(BarsData {
            bars: sorted
                .into_iter()
                .map(|(label, value)| Bar { label, value })
                .collect(),
        }),
    }
}

fn text_lines(sorted: &[(String, u64)]) -> Vec<String> {
    let total: u64 = sorted.iter().map(|(_, v)| v).sum();
    sorted
        .iter()
        .map(|(label, value)| format!("{label}  {}", format_percent(*value, total)))
        .collect()
}

fn markdown_text(sorted: &[(String, u64)]) -> String {
    let total: u64 = sorted.iter().map(|(_, v)| v).sum();
    sorted
        .iter()
        .map(|(label, value)| format!("- **{label}** — {}", format_percent(*value, total)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn text_headline(sorted: &[(String, u64)]) -> String {
    let total: u64 = sorted.iter().map(|(_, v)| v).sum();
    sorted
        .iter()
        .map(|(label, value)| format!("{label} {}", format_percent_short(*value, total)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn format_percent_short(value: u64, total: u64) -> String {
    if total == 0 {
        return "0%".into();
    }
    let pct = (value as f64 / total as f64) * 100.0;
    format!("{pct:.0}%")
}

/// Sort by size descending, cap to `limit` entries. If the source has more languages than
/// the cap, the remainder is folded into a single `"other"` bucket so the totals still add
/// up to the repo's full byte count.
fn top_n(raw: BTreeMap<String, u64>, limit: usize) -> Vec<(String, u64)> {
    let mut all: Vec<(String, u64)> = raw.into_iter().collect();
    all.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    if all.len() <= limit {
        return all;
    }
    let (head, tail) = all.split_at(limit.saturating_sub(1));
    let other: u64 = tail.iter().map(|(_, v)| v).sum();
    let mut out: Vec<(String, u64)> = head.to_vec();
    if other > 0 {
        out.push(("other".into(), other));
    }
    out
}

fn to_entries(sorted: &[(String, u64)]) -> Vec<Entry> {
    let total: u64 = sorted.iter().map(|(_, v)| v).sum();
    sorted
        .iter()
        .map(|(label, value)| Entry {
            key: label.clone(),
            value: Some(format_percent(*value, total)),
            status: None,
        })
        .collect()
}

fn format_percent(value: u64, total: u64) -> String {
    if total == 0 {
        return "0%".into();
    }
    let pct = (value as f64 / total as f64) * 100.0;
    format!("{pct:.1}%")
}

fn repo_for_key(ctx: &FetchContext) -> String {
    ctx.options
        .as_ref()
        .and_then(|v| v.get("repo"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| resolve_repo(None).ok().map(|s: RepoSlug| s.as_path()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| ((*k).into(), *v)).collect()
    }

    #[test]
    fn top_n_sorts_by_size_descending() {
        let input = raw(&[("TOML", 100), ("Rust", 1000), ("Shell", 50)]);
        let sorted = top_n(input, 10);
        assert_eq!(
            sorted,
            vec![
                ("Rust".into(), 1000),
                ("TOML".into(), 100),
                ("Shell".into(), 50)
            ]
        );
    }

    #[test]
    fn top_n_folds_tail_into_other() {
        // 4 items, cap 3 → top 2 kept verbatim, the remaining 2 collapse into "other".
        let input = raw(&[("A", 1000), ("B", 500), ("C", 100), ("D", 50)]);
        let sorted = top_n(input, 3);
        assert_eq!(sorted[0], ("A".into(), 1000));
        assert_eq!(sorted[1], ("B".into(), 500));
        assert_eq!(sorted[2], ("other".into(), 150));
    }

    #[test]
    fn top_n_skips_other_when_no_tail() {
        let input = raw(&[("A", 100), ("B", 50)]);
        let sorted = top_n(input, 5);
        assert_eq!(sorted.len(), 2);
        assert!(sorted.iter().all(|(k, _)| k != "other"));
    }

    #[test]
    fn format_percent_renders_one_decimal() {
        assert_eq!(format_percent(87, 100), "87.0%");
        assert_eq!(format_percent(1, 3), "33.3%");
    }

    #[test]
    fn format_percent_guards_zero_total() {
        assert_eq!(format_percent(0, 0), "0%");
    }

    #[test]
    fn entries_include_percent_values() {
        let sorted = vec![("Rust".into(), 750), ("TOML".into(), 250)];
        let entries = to_entries(&sorted);
        assert_eq!(entries[0].value.as_deref(), Some("75.0%"));
        assert_eq!(entries[1].value.as_deref(), Some("25.0%"));
    }

    #[test]
    fn text_headline_collapses_to_one_line_with_rounded_percents() {
        let sorted = vec![
            ("Rust".into(), 870),
            ("TOML".into(), 80),
            ("Shell".into(), 50),
        ];
        assert_eq!(text_headline(&sorted), "Rust 87% · TOML 8% · Shell 5%");
    }

    #[test]
    fn markdown_text_emits_one_bullet_per_language() {
        let sorted = vec![("Rust".into(), 750), ("TOML".into(), 250)];
        let md = markdown_text(&sorted);
        assert!(md.contains("- **Rust** — 75.0%"));
        assert!(md.contains("- **TOML** — 25.0%"));
    }

    #[test]
    fn build_body_text_emits_text_value() {
        let body = build_body(raw(&[("Rust", 870), ("TOML", 130)]), Shape::Text, 10);
        match body {
            Body::Text(d) => assert!(d.value.contains("Rust") && d.value.contains("87%")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn build_body_bars_preserves_top_n_order() {
        let body = build_body(raw(&[("A", 500), ("B", 1000), ("C", 100)]), Shape::Bars, 10);
        match body {
            Body::Bars(d) => {
                assert_eq!(d.bars[0].label, "B");
                assert_eq!(d.bars[1].label, "A");
                assert_eq!(d.bars[2].label, "C");
            }
            _ => panic!("expected bars"),
        }
    }
}
