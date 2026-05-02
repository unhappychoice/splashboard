//! `code_files` — counts tracked files in the discovered git repo's index and ranks the
//! top-level directories that hold them. `Text` is the headline file count;
//! `TextBlock` / `Entries` / `Bars` / `MarkdownTextBlock` all carry the per-top-level-dir
//! breakdown. Vendored / generated trees are skipped via the family-wide exclusion list.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, TextBlockData,
    TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::git::{open_repo, payload, repo_cache_key};
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::scan::for_each_tracked_path;

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Bars,
    Shape::MarkdownTextBlock,
];
const TOP_LEVEL_LIMIT: usize = 10;
const ROOT_LABEL: &str = "(root)";

pub struct CodeFiles;

#[async_trait]
impl Fetcher for CodeFiles {
    fn name(&self) -> &str {
        "code_files"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Counts tracked files in the discovered git repo's index and ranks top-level directories by file count. `Text` is the headline file count (`\"342 files\"`); `TextBlock` lists `\"<dir> (<count>)\"` rows; `Entries` exposes the same as key/value rows; `Bars` ranks them as bar values; `MarkdownTextBlock` formats them as a Markdown bullet list with emphasis. Files at the repo root are grouped under `\"(root)\"`. Vendored / generated dirs (`node_modules` / `vendor` / `target` / `dist` / `build`) are filtered out even when committed."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Text
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        repo_cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("342 files"),
            Shape::TextBlock => {
                samples::text_block(&["src (180)", "tests (42)", "docs (18)", "(root) (5)"])
            }
            Shape::Entries => samples::entries(&[
                ("src", "180"),
                ("tests", "42"),
                ("docs", "18"),
                ("(root)", "5"),
            ]),
            Shape::Bars => samples::bars(&[("src", 180), ("tests", 42), ("docs", 18)]),
            Shape::MarkdownTextBlock => samples::markdown(
                "- **src** — 180 files\n- **tests** — 42 files\n- **docs** — 18 files",
            ),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let scan = scan_cwd()?;
        Ok(payload(render_body(scan, ctx.shape.unwrap_or(Shape::Text))))
    }
}

#[derive(Debug, Default)]
struct Scan {
    file_count: usize,
    top_level: Vec<(String, u64)>,
}

fn scan_cwd() -> Result<Scan, FetchError> {
    let repo = open_repo()?;
    scan_repo(&repo)
}

fn scan_repo(repo: &gix::Repository) -> Result<Scan, FetchError> {
    let mut file_count = 0usize;
    let mut top_counts: HashMap<String, u64> = HashMap::new();
    for_each_tracked_path(repo, |path| {
        file_count += 1;
        *top_counts.entry(top_level_key(path)).or_insert(0) += 1;
    })?;
    let mut top_level: Vec<_> = top_counts.into_iter().collect();
    top_level.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Scan {
        file_count,
        top_level,
    })
}

fn top_level_key(path: &str) -> String {
    match path.split_once('/') {
        Some((head, _)) => head.to_string(),
        None => ROOT_LABEL.to_string(),
    }
}

fn render_body(scan: Scan, shape: Shape) -> Body {
    if scan.file_count == 0 {
        return empty_body(shape);
    }
    match shape {
        Shape::TextBlock => text_block_body(&scan),
        Shape::Entries => entries_body(&scan),
        Shape::Bars => bars_body(scan),
        Shape::MarkdownTextBlock => markdown_body(&scan),
        _ => text_body(&scan),
    }
}

fn empty_body(shape: Shape) -> Body {
    match shape {
        Shape::TextBlock => Body::TextBlock(TextBlockData { lines: vec![] }),
        Shape::Entries => Body::Entries(EntriesData { items: vec![] }),
        Shape::Bars => Body::Bars(BarsData { bars: vec![] }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: String::new(),
        }),
        _ => Body::Text(TextData {
            value: String::new(),
        }),
    }
}

fn text_body(scan: &Scan) -> Body {
    Body::Text(TextData {
        value: format!("{} {}", scan.file_count, files_word(scan.file_count)),
    })
}

fn text_block_body(scan: &Scan) -> Body {
    Body::TextBlock(TextBlockData {
        lines: top_level_iter(&scan.top_level)
            .map(|(label, count)| format!("{label} ({count})"))
            .collect(),
    })
}

fn entries_body(scan: &Scan) -> Body {
    Body::Entries(EntriesData {
        items: top_level_iter(&scan.top_level)
            .map(|(label, count)| Entry {
                key: label.to_string(),
                value: Some(count.to_string()),
                status: None,
            })
            .collect(),
    })
}

fn bars_body(scan: Scan) -> Body {
    Body::Bars(BarsData {
        bars: scan
            .top_level
            .into_iter()
            .take(TOP_LEVEL_LIMIT)
            .map(|(label, value)| Bar { label, value })
            .collect(),
    })
}

fn markdown_body(scan: &Scan) -> Body {
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: top_level_iter(&scan.top_level)
            .map(|(label, count)| format!("- **{label}** — {count} {}", files_word(count as usize)))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn top_level_iter(top_level: &[(String, u64)]) -> impl Iterator<Item = (&str, u64)> {
    top_level
        .iter()
        .take(TOP_LEVEL_LIMIT)
        .map(|(k, v)| (k.as_str(), *v))
}

fn files_word(n: usize) -> &'static str {
    if n == 1 { "file" } else { "files" }
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
            file_count: 220,
            top_level: vec![
                ("src".into(), 180),
                ("tests".into(), 30),
                ("docs".into(), 9),
                (ROOT_LABEL.into(), 1),
            ],
        }
    }

    #[test]
    fn fetcher_contract_and_samples_cover_supported_shapes() {
        assert_eq!(CodeFiles.name(), "code_files");
        assert!(matches!(CodeFiles.safety(), Safety::Safe));
        assert!(CodeFiles.description().contains("top-level directories"));
        assert_eq!(CodeFiles.default_shape(), Shape::Text);
        assert_eq!(CodeFiles.shapes(), SHAPES);
        assert_ne!(
            CodeFiles.cache_key(&ctx(Some(Shape::Text))),
            CodeFiles.cache_key(&ctx(Some(Shape::Bars)))
        );
        SHAPES.iter().copied().for_each(|shape| {
            let body = CodeFiles.sample_body(shape).expect("sample body");
            assert_eq!(crate::render::shape_of(&body), shape);
        });
        assert!(CodeFiles.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn empty_repo_returns_empty_scan() {
        let (_tmp, repo) = make_repo();
        let scan = scan_repo(&repo).unwrap();
        assert_eq!(scan.file_count, 0);
        assert!(scan.top_level.is_empty());
    }

    #[test]
    fn root_file_groups_under_root_label() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "README.md", "x");
        let scan = scan_repo(&repo).unwrap();
        assert_eq!(scan.file_count, 1);
        assert_eq!(scan.top_level, vec![(ROOT_LABEL.into(), 1)]);
    }

    #[test]
    fn nested_path_attributes_to_top_segment() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "a/b/c/x.rs", "x");
        let scan = scan_repo(&repo).unwrap();
        assert_eq!(scan.file_count, 1);
        assert_eq!(scan.top_level, vec![("a".into(), 1)]);
    }

    #[test]
    fn vendored_dirs_excluded_even_when_committed() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/main.rs", "x");
        commit_touching(&repo, "node_modules/foo/index.js", "y");
        commit_touching(&repo, "vendor/lib/x.go", "z");
        let scan = scan_repo(&repo).unwrap();
        assert_eq!(scan.file_count, 1);
        assert_eq!(scan.top_level[0].0, "src");
    }

    #[test]
    fn top_level_breakdown_ranks_by_count() {
        let (_tmp, repo) = make_repo();
        commit_touching(&repo, "src/a.rs", "1");
        commit_touching(&repo, "src/b.rs", "2");
        commit_touching(&repo, "src/c.rs", "3");
        commit_touching(&repo, "tests/x.rs", "4");
        commit_touching(&repo, "tests/y.rs", "5");
        commit_touching(&repo, "README.md", "6");
        let scan = scan_repo(&repo).unwrap();
        let labels: Vec<_> = scan.top_level.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            labels,
            vec!["src".to_string(), "tests".into(), ROOT_LABEL.into()]
        );
        assert_eq!(scan.top_level[0].1, 3);
        assert_eq!(scan.top_level[1].1, 2);
    }

    #[test]
    fn text_renders_headline_file_count_with_grammar() {
        let one = Scan {
            file_count: 1,
            top_level: vec![],
        };
        assert_eq!(
            render_body(one, Shape::Text),
            Body::Text(TextData {
                value: "1 file".into()
            })
        );
        assert_eq!(
            render_body(fixed_scan(), Shape::Text),
            Body::Text(TextData {
                value: "220 files".into()
            })
        );
    }

    #[test]
    fn text_is_empty_when_no_files() {
        assert_eq!(
            render_body(Scan::default(), Shape::Text),
            Body::Text(TextData {
                value: String::new()
            })
        );
    }

    #[test]
    fn text_block_lists_top_level_dirs() {
        assert_eq!(
            render_body(fixed_scan(), Shape::TextBlock),
            Body::TextBlock(TextBlockData {
                lines: vec![
                    "src (180)".into(),
                    "tests (30)".into(),
                    "docs (9)".into(),
                    "(root) (1)".into(),
                ]
            })
        );
    }

    #[test]
    fn entries_uses_dir_names_as_keys() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Entries),
            Body::Entries(EntriesData {
                items: vec![
                    Entry {
                        key: "src".into(),
                        value: Some("180".into()),
                        status: None,
                    },
                    Entry {
                        key: "tests".into(),
                        value: Some("30".into()),
                        status: None,
                    },
                    Entry {
                        key: "docs".into(),
                        value: Some("9".into()),
                        status: None,
                    },
                    Entry {
                        key: ROOT_LABEL.into(),
                        value: Some("1".into()),
                        status: None,
                    },
                ],
            })
        );
    }

    #[test]
    fn bars_caps_top_level_entries() {
        let top: Vec<_> = (0..(TOP_LEVEL_LIMIT + 5))
            .map(|i| (format!("dir{i}"), 1))
            .collect();
        let scan = Scan {
            file_count: top.len(),
            top_level: top,
        };
        assert!(
            matches!(render_body(scan, Shape::Bars), Body::Bars(BarsData { ref bars }) if bars.len() == TOP_LEVEL_LIMIT)
        );
    }

    #[test]
    fn markdown_emphasises_dir_names_with_count_suffix() {
        assert_eq!(
            render_body(fixed_scan(), Shape::MarkdownTextBlock),
            Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: "- **src** — 180 files\n- **tests** — 30 files\n- **docs** — 9 files\n- **(root)** — 1 file".into()
            })
        );
    }

    #[test]
    fn render_body_uses_text_fallback_and_empty_structural_shapes() {
        assert_eq!(
            render_body(fixed_scan(), Shape::Ratio),
            Body::Text(TextData {
                value: "220 files".into()
            })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::Entries),
            Body::Entries(EntriesData { items: vec![] })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::Bars),
            Body::Bars(BarsData { bars: vec![] })
        );
        assert_eq!(
            render_body(Scan::default(), Shape::MarkdownTextBlock),
            Body::MarkdownTextBlock(MarkdownTextBlockData {
                value: String::new()
            })
        );
    }

    #[test]
    fn empty_text_block_shape_returns_no_lines() {
        assert_eq!(
            render_body(Scan::default(), Shape::TextBlock),
            Body::TextBlock(TextBlockData { lines: vec![] })
        );
    }

    #[tokio::test]
    async fn fetch_scans_current_repo_for_multiple_shapes() {
        let text = CodeFiles.fetch(&ctx(None)).await.unwrap();
        assert!(
            matches!(text.body, Body::Text(TextData { ref value }) if value.ends_with("files") || value.ends_with("file"))
        );

        let entries = CodeFiles.fetch(&ctx(Some(Shape::Entries))).await.unwrap();
        assert!(
            matches!(entries.body, Body::Entries(EntriesData { ref items })
                if !items.is_empty()
                    && !items[0].key.is_empty()
                    && !items[0].value.as_deref().unwrap_or_default().is_empty())
        );

        let markdown = CodeFiles
            .fetch(&ctx(Some(Shape::MarkdownTextBlock)))
            .await
            .unwrap();
        assert!(
            matches!(markdown.body, Body::MarkdownTextBlock(MarkdownTextBlockData { ref value }) if value.contains("**"))
        );
    }
}
