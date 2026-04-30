//! `code_largest_files` — ranks the top tracked files by line count or byte size. Headlines the
//! single biggest file in `Text`; multi-row shapes (`TextBlock` / `Entries` / `Bars` /
//! `MarkdownTextBlock` / `NumberSeries`) carry the per-file ranking. `Badge` flags whether the
//! repo is harbouring a refactor-candidate-sized file (separate health signal from `code_loc`'s
//! repo-total tier).

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, NumberSeriesData,
    Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::for_each_tracked_file;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::NumberSeries,
    Shape::Badge,
];
const DEFAULT_LIMIT: usize = 10;

// Refactor-candidate tiers for `Badge`. Boundaries chosen for the LOC metric (single-file
// review effort): under ~200 lines is comfortable, ~500 starts to feel heavy, ~1000+ is the
// "split this up" zone. Bytes metric reuses the same tier names with KiB-scaled thresholds —
// the qualitative read ("is anything in this repo too big?") stays consistent.
const TIER_LOC_TIDY: u64 = 200;
const TIER_LOC_BIG: u64 = 500;
const TIER_LOC_BLOATED: u64 = 1_000;
const TIER_BYTES_TIDY: u64 = 16 * 1024;
const TIER_BYTES_BIG: u64 = 64 * 1024;
const TIER_BYTES_BLOATED: u64 = 256 * 1024;

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "metric",
        type_hint: "`loc` | `bytes`",
        required: false,
        default: Some("loc"),
        description: "What to rank files by: `loc` (text line count, binaries skipped) or `bytes` (raw file size). LOC suits refactor-candidate hunting; bytes suits asset-weight audits.",
    },
    OptionSchema {
        name: "limit",
        type_hint: "integer",
        required: false,
        default: Some("10"),
        description: "Cap on ranked rows (`TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` / `NumberSeries`). The `Text` headline always names just the single largest file.",
    },
];

/// Ranks the largest tracked source files in the discovered git repo. Walks the `gix` index
/// via the shared family helper, so untracked / `.gitignore`-d files plus committed-vendored
/// trees, lockfiles, and binaries are skipped automatically.
pub struct CodeLargestFiles;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(default)]
    metric: Option<Metric>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Metric {
    #[default]
    Loc,
    Bytes,
}

#[async_trait]
impl Fetcher for CodeLargestFiles {
    fn name(&self) -> &str {
        "code_largest_files"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Ranks the largest tracked source files in the discovered git repo. `metric = \"loc\"` (default) counts text lines per file; `metric = \"bytes\"` measures raw size. `Text` headlines the single biggest file; `TextBlock` / `MarkdownTextBlock` / `Entries` / `Bars` carry the per-file ranking; `NumberSeries` sketches the size distribution; `Badge` flags whether the repo holds a refactor-candidate-sized file (`tidy` / `big` / `bloated` / `monster`). Vendored / generated dirs, lockfiles, and binaries are skipped via the shared scan helper."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
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
            Shape::Text => samples::text("src/render/mod.rs (1,234 LOC)"),
            Shape::TextBlock => samples::text_block(&[
                "src/render/mod.rs (1,234)",
                "src/fetcher/git/mod.rs (812)",
                "src/payload.rs (640)",
            ]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **src/render/mod.rs** — 1,234 LOC\n- **src/fetcher/git/mod.rs** — 812 LOC\n- **src/payload.rs** — 640 LOC",
            ),
            Shape::Entries => samples::entries(&[
                ("src/render/mod.rs", "1,234"),
                ("src/fetcher/git/mod.rs", "812"),
                ("src/payload.rs", "640"),
            ]),
            Shape::Bars => samples::bars(&[
                ("src/render/mod.rs", 1234),
                ("src/fetcher/git/mod.rs", 812),
                ("src/payload.rs", 640),
            ]),
            Shape::NumberSeries => samples::number_series(&[1234, 812, 640, 420, 280]),
            Shape::Badge => samples::badge(Status::Warn, "bloated · src/render/mod.rs (1,234)"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref())?;
        let metric = opts.metric.unwrap_or_default();
        let limit = opts.limit.filter(|n| *n > 0).unwrap_or(DEFAULT_LIMIT);
        let scan = scan_cwd(metric)?;
        Ok(payload(render_body(
            scan,
            ctx.shape.unwrap_or(Shape::Entries),
            metric,
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
struct Scan {
    /// `(path, measure)` sorted by measure desc, then path asc.
    by_file: Vec<(String, u64)>,
}

fn scan_cwd(metric: Metric) -> Result<Scan, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo, metric)
}

fn scan_repo(repo: &gix::Repository, metric: Metric) -> Result<Scan, FetchError> {
    let mut by_file: Vec<(String, u64)> = Vec::new();
    for_each_tracked_file(repo, |path, bytes| {
        let measure = measure_file(metric, bytes);
        if measure == 0 {
            return;
        }
        by_file.push((path.to_string(), measure));
    })?;
    by_file.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Scan { by_file })
}

fn measure_file(metric: Metric, bytes: &[u8]) -> u64 {
    match metric {
        Metric::Bytes => bytes.len() as u64,
        Metric::Loc => match std::str::from_utf8(bytes) {
            Ok(text) => text.lines().count() as u64,
            Err(_) => 0,
        },
    }
}

fn render_body(scan: Scan, shape: Shape, metric: Metric, limit: usize) -> Body {
    if scan.by_file.is_empty() {
        return empty_body(shape);
    }
    match shape {
        Shape::Text => text_body(&scan, metric),
        Shape::TextBlock => text_block_body(&scan, limit),
        Shape::MarkdownTextBlock => markdown_body(&scan, metric, limit),
        Shape::Entries => entries_body(&scan, limit),
        Shape::Bars => bars_body(scan, limit),
        Shape::NumberSeries => number_series_body(&scan, limit),
        Shape::Badge => badge_body(&scan, metric),
        _ => text_body(&scan, metric),
    }
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData { lines: vec![] }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        Shape::Entries => Body::Entries(EntriesData { items: vec![] }),
        Shape::Bars => Body::Bars(BarsData { bars: vec![] }),
        Shape::NumberSeries => Body::NumberSeries(NumberSeriesData { values: vec![] }),
        Shape::Badge => Body::Badge(BadgeData {
            status: Status::Ok,
            label: "empty".into(),
        }),
        _ => Body::Text(TextData {
            value: String::new(),
        }),
    }
}

fn text_body(scan: &Scan, metric: Metric) -> Body {
    let (path, measure) = scan.by_file.first().expect("non-empty scan");
    Body::Text(TextData {
        value: format!("{path} ({})", format_measure(*measure, metric)),
    })
}

fn text_block_body(scan: &Scan, limit: usize) -> Body {
    Body::TextBlock(TextBlockData {
        lines: scan
            .by_file
            .iter()
            .take(limit)
            .map(|(path, n)| format!("{path} ({})", format_with_commas(*n)))
            .collect(),
    })
}

fn markdown_body(scan: &Scan, metric: Metric, limit: usize) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: scan
            .by_file
            .iter()
            .take(limit)
            .map(|(path, n)| format!("- **{path}** — {}", format_measure(*n, metric)))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn entries_body(scan: &Scan, limit: usize) -> Body {
    Body::Entries(EntriesData {
        items: scan
            .by_file
            .iter()
            .take(limit)
            .map(|(path, n)| Entry {
                key: path.clone(),
                value: Some(format_with_commas(*n)),
                status: None,
            })
            .collect(),
    })
}

fn bars_body(scan: Scan, limit: usize) -> Body {
    Body::Bars(BarsData {
        bars: scan
            .by_file
            .into_iter()
            .take(limit)
            .map(|(path, n)| Bar {
                label: path,
                value: n,
            })
            .collect(),
    })
}

fn number_series_body(scan: &Scan, limit: usize) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: scan.by_file.iter().take(limit).map(|(_, n)| *n).collect(),
    })
}

fn badge_body(scan: &Scan, metric: Metric) -> Body {
    let (path, measure) = scan.by_file.first().expect("non-empty scan");
    let (tier, status) = tier_for(metric, *measure);
    Body::Badge(BadgeData {
        status,
        label: format!("{tier} · {path} ({})", format_with_commas(*measure)),
    })
}

fn tier_for(metric: Metric, measure: u64) -> (&'static str, Status) {
    let (tidy, big, bloated) = match metric {
        Metric::Loc => (TIER_LOC_TIDY, TIER_LOC_BIG, TIER_LOC_BLOATED),
        Metric::Bytes => (TIER_BYTES_TIDY, TIER_BYTES_BIG, TIER_BYTES_BLOATED),
    };
    if measure < tidy {
        ("tidy", Status::Ok)
    } else if measure < big {
        ("big", Status::Ok)
    } else if measure < bloated {
        ("bloated", Status::Warn)
    } else {
        ("monster", Status::Error)
    }
}

fn format_measure(n: u64, metric: Metric) -> String {
    let suffix = match metric {
        Metric::Loc => "LOC",
        Metric::Bytes => "bytes",
    };
    format!("{} {suffix}", format_with_commas(n))
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

    fn ctx(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "widget".into(),
            shape,
            ..Default::default()
        }
    }

    fn fixed_scan() -> Scan {
        Scan {
            by_file: vec![
                ("src/render/mod.rs".into(), 1_234),
                ("src/fetcher/git/mod.rs".into(), 812),
                ("src/payload.rs".into(), 640),
            ],
        }
    }

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        assert_eq!(CodeLargestFiles.name(), "code_largest_files");
        assert!(matches!(CodeLargestFiles.safety(), Safety::Safe));
        assert!(
            CodeLargestFiles
                .description()
                .contains("largest tracked source files")
        );
        assert_eq!(CodeLargestFiles.default_shape(), Shape::Entries);
        assert_eq!(CodeLargestFiles.shapes(), SHAPES);
        assert_eq!(
            CodeLargestFiles.option_schemas().len(),
            OPTION_SCHEMAS.len()
        );
        SHAPES.iter().copied().for_each(|shape| {
            let body = CodeLargestFiles.sample_body(shape).expect("sample body");
            assert_eq!(crate::render::shape_of(&body), shape);
        });
        assert!(CodeLargestFiles.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn empty_repo_returns_empty_scan() {
        let (_tmp, repo) = make_repo();
        let scan = scan_repo(&repo, Metric::Loc).unwrap();
        assert!(scan.by_file.is_empty());
    }

    #[test]
    fn ranks_files_by_loc_descending() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "small.rs", "a");
        commit_touching(&repo, "big.rs", "1\n2\n3\n4\n5");
        commit_touching(&repo, "mid.rs", "x\ny\nz");
        let scan = scan_repo(&repo, Metric::Loc).unwrap();
        let names: Vec<_> = scan.by_file.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(names, vec!["big.rs", "mid.rs", "small.rs"]);
    }

    #[test]
    fn lockfiles_and_vendored_excluded() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "fn main() {}");
        commit_touching(
            &repo,
            "Cargo.lock",
            "noise\nnoise\nnoise\nnoise\nnoise\nnoise",
        );
        commit_touching(&repo, "node_modules/big.js", "x\ny\nz\nw\nq\nr");
        let scan = scan_repo(&repo, Metric::Loc).unwrap();
        let paths: Vec<_> = scan.by_file.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["src/main.rs"]);
    }

    #[test]
    fn bytes_metric_uses_raw_file_size() {
        let (_tmp, repo) = make_repo();
        // shorter content, more lines
        commit_touching(&repo, "many.rs", "a\nb\nc\nd\ne");
        // longer content, fewer lines
        commit_touching(&repo, "long.rs", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
        let scan = scan_repo(&repo, Metric::Bytes).unwrap();
        let head = scan.by_file.first().map(|(p, _)| p.as_str());
        assert_eq!(head, Some("long.rs"));
    }

    #[test]
    fn text_headlines_largest_file() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Text, Metric::Loc, 10),
            Body::Text(TextData {
                value: "src/render/mod.rs (1,234 LOC)".into(),
            })
        );
    }

    #[test]
    fn text_uses_metric_suffix() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Text, Metric::Bytes, 10),
            Body::Text(TextData {
                value: "src/render/mod.rs (1,234 bytes)".into(),
            })
        );
    }

    #[test]
    fn text_is_empty_when_no_files() {
        assert_eq!(
            render_body(Scan::default(), Shape::Text, Metric::Loc, 10),
            Body::Text(TextData {
                value: String::new(),
            })
        );
    }

    #[test]
    fn text_block_lists_ranked_files() {
        assert_eq!(
            render_body(fixed_scan(), Shape::TextBlock, Metric::Loc, 10),
            Body::TextBlock(TextBlockData {
                lines: vec![
                    "src/render/mod.rs (1,234)".into(),
                    "src/fetcher/git/mod.rs (812)".into(),
                    "src/payload.rs (640)".into(),
                ],
            })
        );
    }

    #[test]
    fn entries_carry_path_and_count() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Entries, Metric::Loc, 10),
            Body::Entries(EntriesData {
                items: vec![
                    Entry {
                        key: "src/render/mod.rs".into(),
                        value: Some("1,234".into()),
                        status: None,
                    },
                    Entry {
                        key: "src/fetcher/git/mod.rs".into(),
                        value: Some("812".into()),
                        status: None,
                    },
                    Entry {
                        key: "src/payload.rs".into(),
                        value: Some("640".into()),
                        status: None,
                    },
                ],
            })
        );
    }

    #[test]
    fn bars_carry_path_and_raw_value() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Bars, Metric::Loc, 10),
            Body::Bars(BarsData {
                bars: vec![
                    Bar {
                        label: "src/render/mod.rs".into(),
                        value: 1234,
                    },
                    Bar {
                        label: "src/fetcher/git/mod.rs".into(),
                        value: 812,
                    },
                    Bar {
                        label: "src/payload.rs".into(),
                        value: 640,
                    },
                ],
            })
        );
    }

    #[test]
    fn markdown_emphasises_path_with_metric_suffix() {
        assert_eq!(
            render_body(fixed_scan(), Shape::MarkdownTextBlock, Metric::Loc, 10),
            Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: [
                    "- **src/render/mod.rs** — 1,234 LOC",
                    "- **src/fetcher/git/mod.rs** — 812 LOC",
                    "- **src/payload.rs** — 640 LOC",
                ]
                .join("\n"),
            })
        );
    }

    #[test]
    fn number_series_emits_sorted_measures() {
        assert_eq!(
            render_body(fixed_scan(), Shape::NumberSeries, Metric::Loc, 10),
            Body::NumberSeries(NumberSeriesData {
                values: vec![1234, 812, 640],
            })
        );
    }

    #[test]
    fn limit_caps_rendered_rows() {
        let Body::Entries(d) = render_body(fixed_scan(), Shape::Entries, Metric::Loc, 2) else {
            unreachable!();
        };
        assert_eq!(d.items.len(), 2);
    }

    #[test]
    fn badge_tiers_by_largest_file_size_loc() {
        assert_eq!(tier_for(Metric::Loc, 50), ("tidy", Status::Ok));
        assert_eq!(tier_for(Metric::Loc, 300), ("big", Status::Ok));
        assert_eq!(tier_for(Metric::Loc, 700), ("bloated", Status::Warn));
        assert_eq!(tier_for(Metric::Loc, 5_000), ("monster", Status::Error));
    }

    #[test]
    fn badge_tiers_by_largest_file_size_bytes() {
        assert_eq!(tier_for(Metric::Bytes, 1024), ("tidy", Status::Ok));
        assert_eq!(tier_for(Metric::Bytes, 32 * 1024), ("big", Status::Ok));
        assert_eq!(
            tier_for(Metric::Bytes, 100 * 1024),
            ("bloated", Status::Warn)
        );
        assert_eq!(
            tier_for(Metric::Bytes, 500 * 1024),
            ("monster", Status::Error)
        );
    }

    #[test]
    fn badge_label_includes_tier_path_and_count() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Badge, Metric::Loc, 10),
            Body::Badge(BadgeData {
                status: Status::Error,
                label: "monster · src/render/mod.rs (1,234)".into(),
            })
        );
    }

    #[test]
    fn badge_handles_empty() {
        assert_eq!(
            render_body(Scan::default(), Shape::Badge, Metric::Loc, 10),
            Body::Badge(BadgeData {
                status: Status::Ok,
                label: "empty".into(),
            })
        );
    }

    #[test]
    fn render_body_covers_empty_variants_and_text_fallback() {
        assert_eq!(
            render_body(Scan::default(), Shape::TextBlock, Metric::Loc, 10),
            Body::TextBlock(TextBlockData { lines: vec![] })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::MarkdownTextBlock, Metric::Loc, 10),
            Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: String::new(),
            })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::Entries, Metric::Loc, 10),
            Body::Entries(EntriesData { items: vec![] })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::Bars, Metric::Loc, 10),
            Body::Bars(BarsData { bars: vec![] })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::NumberSeries, Metric::Loc, 10),
            Body::NumberSeries(NumberSeriesData { values: vec![] })
        );
        assert_eq!(
            render_body(fixed_scan(), Shape::Ratio, Metric::Bytes, 10),
            Body::Text(TextData {
                value: "src/render/mod.rs (1,234 bytes)".into(),
            })
        );
    }

    #[test]
    fn cache_key_changes_with_options() {
        let mut ctx = FetchContext {
            widget_id: "w".into(),
            ..Default::default()
        };
        let k0 = CodeLargestFiles.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"limit = 5"#).unwrap());
        let k1 = CodeLargestFiles.cache_key(&ctx);
        ctx.options = Some(toml::from_str(r#"metric = "bytes""#).unwrap());
        let k2 = CodeLargestFiles.cache_key(&ctx);
        assert_ne!(k0, k1);
        assert_ne!(k1, k2);
    }

    #[test]
    fn parse_options_rejects_unknown_keys() {
        let bad: toml::Value = toml::from_str(r#"unknown = 1"#).unwrap();
        assert!(parse_options(Some(&bad)).is_err());
    }

    #[test]
    fn parse_options_accepts_metric_variants() {
        let loc: toml::Value = toml::from_str(r#"metric = "loc""#).unwrap();
        assert_eq!(parse_options(Some(&loc)).unwrap().metric, Some(Metric::Loc));
        let bytes: toml::Value = toml::from_str(r#"metric = "bytes""#).unwrap();
        assert_eq!(
            parse_options(Some(&bytes)).unwrap().metric,
            Some(Metric::Bytes)
        );
    }

    #[test]
    fn parse_options_defaults_and_loc_metric_skips_invalid_utf8() {
        let opts = parse_options(None).unwrap();
        assert_eq!(opts.metric, None);
        assert_eq!(opts.limit, None);
        assert_eq!(measure_file(Metric::Loc, &[0xff, 0xfe]), 0);
    }

    #[test]
    fn fetch_reads_cwd_repo_for_default_and_requested_shapes() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (_tmp, repo) = crate::fetcher::git::test_support::make_repo();
        crate::fetcher::git::test_support::commit_touching(
            &repo,
            "src/lib.rs",
            "pub fn hello() {}\n",
        );
        let workdir = repo.workdir().unwrap().to_path_buf();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&workdir).unwrap();

        let entries = rt.block_on(CodeLargestFiles.fetch(&ctx(None)));

        let mut number_series_ctx = ctx(Some(Shape::NumberSeries));
        number_series_ctx.options = Some(toml::from_str("metric = \"bytes\"\nlimit = 1").unwrap());
        let number_series = rt.block_on(CodeLargestFiles.fetch(&number_series_ctx));

        let mut default_limit_ctx = ctx(Some(Shape::NumberSeries));
        default_limit_ctx.options = Some(toml::from_str("limit = 0").unwrap());
        let default_limit = rt.block_on(CodeLargestFiles.fetch(&default_limit_ctx));

        let expected_entries = scan_repo(&repo, Metric::Loc)
            .map(|s| render_body(s, Shape::Entries, Metric::Loc, DEFAULT_LIMIT));
        let expected_bytes = scan_repo(&repo, Metric::Bytes)
            .map(|s| render_body(s, Shape::NumberSeries, Metric::Bytes, 1));
        let expected_default_limit = scan_repo(&repo, Metric::Loc)
            .map(|s| render_body(s, Shape::NumberSeries, Metric::Loc, DEFAULT_LIMIT));

        std::env::set_current_dir(prev_cwd).unwrap();

        assert_eq!(entries.unwrap().body, expected_entries.unwrap());
        assert_eq!(number_series.unwrap().body, expected_bytes.unwrap());
        assert_eq!(default_limit.unwrap().body, expected_default_limit.unwrap());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options() {
        let mut bad = ctx(None);
        bad.options = Some(toml::from_str("unknown = 1").unwrap());
        let err = CodeLargestFiles.fetch(&bad).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("invalid options")));
    }
}
