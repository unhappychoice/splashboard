//! `huggingface_trending` — trending models / datasets / spaces on Hugging Face Hub.
//!
//! Safety::Safe: API host (`huggingface.co`) is hardcoded; the user supplies only `kind`
//! (closed enum) and `count`. Thumbnails are fetched as a best-effort per-author lookup
//! against the same fixed host; failed lookups silently leave a row's thumbnail empty
//! rather than poisoning the whole feed.
//!
//! HF's `?sort=trendingScore&direction=-1` is the documented "Trending" sort on the public
//! Hub API (no auth required), so this fetcher mirrors what huggingface.co/{models,datasets,
//! spaces} shows under the "Trending" tab.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    Bar, BarsData, Body, EntriesData, Entry, ImageLinkedItem, ImageLinkedListData, LinkedLine,
    LinkedTextBlockData, MarkdownTextBlockData, Payload, TextBlockData, TextData,
};
use crate::render::Shape;

use super::github::common::{cache_key, parse_options, payload};
use super::thumbnails;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_BASE: &str = "https://huggingface.co/api";
const SITE_BASE: &str = "https://huggingface.co";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 5 * 1024 * 1024;

const DEFAULT_COUNT: u32 = 10;
const MIN_COUNT: u32 = 1;
const MAX_COUNT: u32 = 30;

const SHAPES: &[Shape] = &[
    Shape::LinkedTextBlock,
    Shape::ImageLinkedList,
    Shape::TextBlock,
    Shape::Text,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Bars,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "kind",
        type_hint: "\"models\" | \"datasets\" | \"spaces\"",
        required: false,
        default: Some("\"models\""),
        description: "Which Hub catalog to rank.",
    },
    OptionSchema {
        name: "count",
        type_hint: "integer (1..=30)",
        required: false,
        default: Some("10"),
        description: "Number of trending entries to display.",
    },
];

pub struct HuggingfaceTrendingFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub kind: Option<Kind>,
    #[serde(default)]
    pub count: Option<u32>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Models,
    Datasets,
    Spaces,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Self::Models => "models",
            Self::Datasets => "datasets",
            Self::Spaces => "spaces",
        }
    }

    /// Path used to build a clickable Hub page URL — same as the API path for all three.
    fn site_path(self) -> &'static str {
        self.path()
    }
}

#[async_trait]
impl Fetcher for HuggingfaceTrendingFetcher {
    fn name(&self) -> &str {
        "huggingface_trending"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Trending models, datasets, or spaces on Hugging Face Hub via the public `?sort=trendingScore` API (no auth required). Companion to the planned `github_trending` / `wikipedia_trending` family — each surface picks its own list, this one is HF's."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 60 * 6
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        sample_trending_body(shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: Options = parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let kind = opts.kind.unwrap_or_default();
        let count = opts
            .count
            .unwrap_or(DEFAULT_COUNT)
            .clamp(MIN_COUNT, MAX_COUNT) as usize;
        let shape = ctx.shape.unwrap_or(Shape::LinkedTextBlock);
        let entries = fetch_trending(kind, count).await?;
        let body = render_for_shape(&entries, kind, shape).await;
        Ok(payload(body))
    }
}

/// Trending Hub entry as the renderer wants it. `position` is the 0-indexed slot in HF's
/// response (= trending rank); HF's own `trendingScore` is a float we display rather than
/// re-rank on. `id` is the canonical `author/repo` slug — we split off `author` for the
/// per-author avatar lookup that powers `ImageLinkedList`.
#[derive(Debug, Clone)]
struct Entrt {
    id: String,
    likes: u64,
    downloads: Option<u64>,
    pipeline_tag: Option<String>,
    position: usize,
}

impl Entrt {
    fn author(&self) -> Option<&str> {
        self.id.split_once('/').map(|(a, _)| a)
    }

    fn url(&self, kind: Kind) -> String {
        match kind {
            // Models live at the bare `/{id}` slug; datasets / spaces are namespaced.
            Kind::Models => format!("{SITE_BASE}/{}", self.id),
            other => format!("{SITE_BASE}/{}/{}", other.site_path(), self.id),
        }
    }

    fn score_line(&self) -> String {
        let likes = format_count(self.likes);
        match self.downloads {
            Some(d) if d > 0 => format!("♥ {likes}  ↓ {}", format_count(d)),
            _ => format!("♥ {likes}"),
        }
    }
}

async fn fetch_trending(kind: Kind, limit: usize) -> Result<Vec<Entrt>, FetchError> {
    let url = format!(
        "{API_BASE}/{}?sort=trendingScore&direction=-1&limit={limit}",
        kind.path()
    );
    let raw: Vec<ApiEntry> = get_json(&url).await?;
    Ok(raw
        .into_iter()
        .enumerate()
        .map(|(i, e)| Entrt {
            id: e.id,
            likes: e.likes.unwrap_or(0).max(0) as u64,
            downloads: e.downloads.map(|d| d.max(0) as u64),
            pipeline_tag: e.pipeline_tag,
            position: i,
        })
        .collect())
}

async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, FetchError> {
    let res = http()
        .get(url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("huggingface request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("huggingface {status}")));
    }
    let bytes = res
        .bytes()
        .await
        .map_err(|e| FetchError::Failed(format!("huggingface read body: {e}")))?;
    if bytes.len() > MAX_BYTES {
        return Err(FetchError::Failed(format!(
            "huggingface response too large ({} bytes, cap {MAX_BYTES})",
            bytes.len()
        )));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| FetchError::Failed(format!("huggingface json parse: {e}")))
}

fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

async fn render_for_shape(entries: &[Entrt], kind: Kind, shape: Shape) -> Body {
    if matches!(shape, Shape::ImageLinkedList) {
        let paths = resolve_thumbnails(entries).await;
        return image_linked_body(entries, kind, &paths);
    }
    render_sync(entries, kind, shape)
}

/// Avatar URLs aren't carried in the trending response, so resolve them per-author via the
/// `overview` endpoint. Authors repeat across rows (e.g. `meta-llama` ships several trending
/// models at once) — in-loop dedup so a 20-row feed doesn't fan out 20 lookups when 4
/// distinct authors are involved.
async fn resolve_thumbnails(entries: &[Entrt]) -> Vec<Option<PathBuf>> {
    let mut cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut urls: Vec<Option<String>> = Vec::with_capacity(entries.len());
    for entry in entries {
        let avatar = match entry.author() {
            Some(author) => match cache.get(author) {
                Some(cached) => cached.clone(),
                None => {
                    let resolved = fetch_avatar_url(author).await;
                    cache.insert(author.to_string(), resolved.clone());
                    resolved
                }
            },
            None => None,
        };
        urls.push(avatar);
    }
    thumbnails::download_many(&urls).await
}

/// HF puts users and organizations in the same namespace but exposes them via separate API
/// paths. Try the user endpoint first; on any error fall through to the org endpoint. Either
/// way the failure is silent — a missing avatar shouldn't break the row.
async fn fetch_avatar_url(author: &str) -> Option<String> {
    for kind in ["users", "organizations"] {
        let url = format!("{API_BASE}/{kind}/{author}/overview");
        if let Ok(profile) = get_json::<HfProfile>(&url).await
            && let Some(avatar) = profile.avatar_url.filter(|s| !s.is_empty())
        {
            return Some(avatar);
        }
    }
    None
}

fn render_sync(entries: &[Entrt], kind: Kind, shape: Shape) -> Body {
    match shape {
        Shape::Text => Body::Text(TextData {
            value: headline(entries),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: entries.iter().map(row_line).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: markdown_body(entries, kind),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: entries.iter().map(entry_row).collect(),
        }),
        Shape::Bars => Body::Bars(BarsData {
            bars: entries.iter().map(likes_bar).collect(),
        }),
        _ => Body::LinkedTextBlock(LinkedTextBlockData {
            items: entries
                .iter()
                .map(|e| LinkedLine {
                    text: row_line(e),
                    url: Some(e.url(kind)),
                })
                .collect(),
        }),
    }
}

fn image_linked_body(entries: &[Entrt], kind: Kind, paths: &[Option<PathBuf>]) -> Body {
    Body::ImageLinkedList(ImageLinkedListData {
        items: entries
            .iter()
            .enumerate()
            .map(|(i, e)| ImageLinkedItem {
                title: format!("#{} {}", e.position + 1, e.id),
                url: Some(e.url(kind)),
                thumbnail_path: paths
                    .get(i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.to_string_lossy().into_owned()),
                subtitle: Some(subtitle_line(e)),
            })
            .collect(),
    })
}

fn headline(entries: &[Entrt]) -> String {
    entries
        .first()
        .map(|e| format!("#1 {}  {}", e.id, e.score_line()))
        .unwrap_or_else(|| "(no trending entries)".into())
}

fn row_line(entry: &Entrt) -> String {
    format!(
        "#{} {}  {}",
        entry.position + 1,
        entry.id,
        entry.score_line()
    )
}

fn subtitle_line(entry: &Entrt) -> String {
    let mut s = entry.score_line();
    if let Some(tag) = &entry.pipeline_tag {
        s.push_str(&format!("  · {tag}"));
    }
    s
}

fn markdown_body(entries: &[Entrt], kind: Kind) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                "- **#{} {}** — {}  ([page]({}))",
                e.position + 1,
                e.id,
                e.score_line(),
                e.url(kind),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_row(entry: &Entrt) -> Entry {
    Entry {
        key: format!("#{} {}", entry.position + 1, entry.id),
        value: Some(entry.score_line()),
        status: None,
    }
}

fn likes_bar(entry: &Entrt) -> Bar {
    Bar {
        label: entry.id.clone(),
        value: entry.likes,
        value_label: None,
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn sample_trending_body(shape: Shape) -> Option<Body> {
    Some(match shape {
        Shape::LinkedTextBlock => crate::samples::linked_text_block(&[
            (
                "#1 meta-llama/Llama-3.1-8B-Instruct  ♥ 3.2k  ↓ 2.1M",
                Some("https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct"),
            ),
            (
                "#2 Qwen/Qwen2.5-72B-Instruct  ♥ 1.8k  ↓ 850.0k",
                Some("https://huggingface.co/Qwen/Qwen2.5-72B-Instruct"),
            ),
        ]),
        Shape::ImageLinkedList => crate::samples::image_linked_list(&[
            (
                "#1 meta-llama/Llama-3.1-8B-Instruct",
                Some("https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct"),
                None,
                Some("♥ 3.2k  ↓ 2.1M  · text-generation"),
            ),
            (
                "#2 Qwen/Qwen2.5-72B-Instruct",
                Some("https://huggingface.co/Qwen/Qwen2.5-72B-Instruct"),
                None,
                Some("♥ 1.8k  ↓ 850.0k  · text-generation"),
            ),
        ]),
        Shape::TextBlock => crate::samples::text_block(&[
            "#1 meta-llama/Llama-3.1-8B-Instruct  ♥ 3.2k  ↓ 2.1M",
            "#2 Qwen/Qwen2.5-72B-Instruct  ♥ 1.8k  ↓ 850.0k",
        ]),
        Shape::Text => crate::samples::text("#1 meta-llama/Llama-3.1-8B-Instruct  ♥ 3.2k  ↓ 2.1M"),
        Shape::MarkdownTextBlock => crate::samples::markdown(
            "- **#1 meta-llama/Llama-3.1-8B-Instruct** — ♥ 3.2k  ↓ 2.1M  ([page](https://huggingface.co/meta-llama/Llama-3.1-8B-Instruct))\n- **#2 Qwen/Qwen2.5-72B-Instruct** — ♥ 1.8k  ↓ 850.0k  ([page](https://huggingface.co/Qwen/Qwen2.5-72B-Instruct))",
        ),
        Shape::Entries => crate::samples::entries(&[
            ("#1 meta-llama/Llama-3.1-8B-Instruct", "♥ 3.2k  ↓ 2.1M"),
            ("#2 Qwen/Qwen2.5-72B-Instruct", "♥ 1.8k  ↓ 850.0k"),
        ]),
        Shape::Bars => crate::samples::bars(&[
            ("meta-llama/Llama-3.1-8B-Instruct", 3_200),
            ("Qwen/Qwen2.5-72B-Instruct", 1_800),
        ]),
        _ => return None,
    })
}

#[derive(Debug, Deserialize)]
struct ApiEntry {
    id: String,
    #[serde(default)]
    likes: Option<i64>,
    #[serde(default)]
    downloads: Option<i64>,
    #[serde(default)]
    pipeline_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HfProfile {
    #[serde(rename = "avatarUrl", default)]
    avatar_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration as StdDuration;

    use super::*;

    fn ctx(options: Option<&str>, shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "hf-trending".into(),
            timeout: StdDuration::from_secs(1),
            shape,
            options: options.map(|raw| toml::from_str(raw).unwrap()),
            ..Default::default()
        }
    }

    fn entry(id: &str, position: usize, likes: u64, downloads: Option<u64>) -> Entrt {
        Entrt {
            id: id.into(),
            likes,
            downloads,
            pipeline_tag: Some("text-generation".into()),
            position,
        }
    }

    #[test]
    fn kind_paths_cover_all_variants() {
        assert_eq!(Kind::Models.path(), "models");
        assert_eq!(Kind::Datasets.path(), "datasets");
        assert_eq!(Kind::Spaces.path(), "spaces");
    }

    #[test]
    fn options_default_to_none() {
        let opts = Options::default();
        assert!(opts.kind.is_none());
        assert!(opts.count.is_none());
    }

    #[test]
    fn options_deserialize_full() {
        let raw: toml::Value = toml::from_str("kind = \"datasets\"\ncount = 5").unwrap();
        let opts: Options = raw.try_into().unwrap();
        assert_eq!(opts.kind, Some(Kind::Datasets));
        assert_eq!(opts.count, Some(5));
    }

    #[test]
    fn options_reject_unknown_keys() {
        let raw: toml::Value = toml::from_str("bogus = true").unwrap();
        assert!(raw.try_into::<Options>().is_err());
    }

    #[test]
    fn entry_author_splits_on_first_slash() {
        let e = entry("meta-llama/Llama-3.1", 0, 0, None);
        assert_eq!(e.author(), Some("meta-llama"));
    }

    #[test]
    fn entry_author_handles_idless_namespace() {
        let e = entry("no_slash", 0, 0, None);
        assert_eq!(e.author(), None);
    }

    #[test]
    fn entry_url_uses_bare_slug_for_models_and_namespaced_for_others() {
        let e = entry("author/repo", 0, 0, None);
        assert_eq!(e.url(Kind::Models), "https://huggingface.co/author/repo");
        assert_eq!(
            e.url(Kind::Datasets),
            "https://huggingface.co/datasets/author/repo"
        );
        assert_eq!(
            e.url(Kind::Spaces),
            "https://huggingface.co/spaces/author/repo"
        );
    }

    #[test]
    fn entry_score_line_includes_downloads_when_present() {
        let with = entry("a/b", 0, 3200, Some(2_100_000));
        assert_eq!(with.score_line(), "♥ 3.2k  ↓ 2.1M");
        let without = entry("a/b", 0, 100, None);
        assert_eq!(without.score_line(), "♥ 100");
        let zero_dl = entry("a/b", 0, 100, Some(0));
        assert_eq!(zero_dl.score_line(), "♥ 100");
    }

    #[test]
    fn format_count_picks_unit_by_magnitude() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(950), "950");
        assert_eq!(format_count(1_500), "1.5k");
        assert_eq!(format_count(1_200_000), "1.2M");
    }

    #[test]
    fn render_sync_text_uses_first_entry_with_rank_prefix() {
        let body = render_sync(
            &[entry("meta-llama/Llama-3.1", 0, 3200, Some(2_100_000))],
            Kind::Models,
            Shape::Text,
        );
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert!(t.value.starts_with("#1"));
        assert!(t.value.contains("meta-llama/Llama-3.1"));
    }

    #[test]
    fn render_sync_text_handles_empty_entries() {
        let body = render_sync(&[], Kind::Models, Shape::Text);
        let Body::Text(t) = body else {
            panic!("expected text");
        };
        assert_eq!(t.value, "(no trending entries)");
    }

    #[test]
    fn render_sync_text_block_lists_one_rank_prefixed_line_per_entry() {
        let entries = vec![
            entry("meta-llama/Llama-3.1", 0, 3200, Some(2_100_000)),
            entry("author/repo", 1, 100, None),
        ];
        let body = render_sync(&entries, Kind::Models, Shape::TextBlock);
        let Body::TextBlock(b) = body else {
            panic!("expected text_block");
        };
        assert_eq!(b.lines.len(), 2);
        assert_eq!(b.lines[0], "#1 meta-llama/Llama-3.1  ♥ 3.2k  ↓ 2.1M");
        assert_eq!(b.lines[1], "#2 author/repo  ♥ 100");
    }

    #[test]
    fn render_sync_linked_text_block_uses_kind_aware_url() {
        let body = render_sync(
            &[entry("author/repo", 0, 100, None)],
            Kind::Datasets,
            Shape::LinkedTextBlock,
        );
        let Body::LinkedTextBlock(b) = body else {
            panic!("expected linked_text_block");
        };
        assert_eq!(
            b.items[0].url.as_deref(),
            Some("https://huggingface.co/datasets/author/repo")
        );
    }

    #[test]
    fn render_sync_bars_carry_likes_as_value() {
        let entries = vec![entry("a/b", 0, 3200, None), entry("c/d", 1, 1800, None)];
        let body = render_sync(&entries, Kind::Models, Shape::Bars);
        let Body::Bars(b) = body else {
            panic!("expected bars");
        };
        assert_eq!(b.bars[0].label, "a/b");
        assert_eq!(b.bars[0].value, 3200);
        assert_eq!(b.bars[1].value, 1800);
    }

    #[test]
    fn render_sync_markdown_includes_page_link() {
        let body = render_sync(
            &[entry("author/repo", 0, 100, None)],
            Kind::Models,
            Shape::MarkdownTextBlock,
        );
        let Body::MarkdownTextBlock(m) = body else {
            panic!("expected markdown");
        };
        assert!(m.value.contains("[page]("));
        assert!(m.value.contains("author/repo"));
    }

    #[test]
    fn render_sync_entries_use_rank_prefixed_key_and_score_value() {
        let body = render_sync(
            &[entry("author/repo", 0, 3200, Some(2_100_000))],
            Kind::Models,
            Shape::Entries,
        );
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items[0].key, "#1 author/repo");
        assert_eq!(e.items[0].value.as_deref(), Some("♥ 3.2k  ↓ 2.1M"));
    }

    #[test]
    fn image_linked_body_pins_paths_and_subtitle_with_pipeline_tag() {
        let entries = vec![entry("author/repo", 0, 100, Some(50))];
        let paths = vec![Some(PathBuf::from("/tmp/a.png"))];
        let body = image_linked_body(&entries, Kind::Models, &paths);
        let Body::ImageLinkedList(d) = body else {
            panic!("expected image_linked_list");
        };
        assert_eq!(d.items[0].title, "#1 author/repo");
        assert_eq!(d.items[0].thumbnail_path.as_deref(), Some("/tmp/a.png"));
        let subtitle = d.items[0].subtitle.as_deref().unwrap();
        assert!(subtitle.contains("text-generation"));
    }

    #[test]
    fn fetcher_exposes_catalog_metadata_and_samples() {
        let fetcher = HuggingfaceTrendingFetcher;
        assert_eq!(fetcher.name(), "huggingface_trending");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert_eq!(fetcher.shapes(), SHAPES);
        assert_eq!(fetcher.default_shape(), Shape::LinkedTextBlock);
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["kind", "count"]
        );
        for shape in SHAPES {
            assert!(
                fetcher.sample_body(*shape).is_some(),
                "missing sample for {shape:?}"
            );
        }
        assert!(fetcher.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn cache_key_partitions_by_shape_and_options() {
        let fetcher = HuggingfaceTrendingFetcher;
        let linked = fetcher.cache_key(&ctx(None, Some(Shape::LinkedTextBlock)));
        let bars = fetcher.cache_key(&ctx(None, Some(Shape::Bars)));
        let datasets = fetcher.cache_key(&ctx(
            Some("kind = \"datasets\""),
            Some(Shape::LinkedTextBlock),
        ));
        assert_ne!(linked, bars);
        assert_ne!(linked, datasets);
    }

    #[tokio::test]
    async fn fetch_rejects_unknown_options_before_network() {
        let err = HuggingfaceTrendingFetcher
            .fetch(&ctx(Some("bogus = true"), Some(Shape::Text)))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("unknown field"));
    }

    fn serve_once(status: &str, body: &str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_owned();
        let body = body.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn get_json_deserializes_success_body() {
        let (url, server) = serve_once("200 OK", r#"[{"id":"meta/llama","likes":3}]"#);
        let entries: Vec<ApiEntry> = get_json(&url).await.unwrap();
        server.join().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "meta/llama");
        assert_eq!(entries[0].likes, Some(3));
    }

    #[tokio::test]
    async fn get_json_surfaces_non_success_status() {
        let (url, server) = serve_once("503 Service Unavailable", "");
        let err = get_json::<Vec<ApiEntry>>(&url).await.unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg == "huggingface 503 Service Unavailable"
        ));
    }

    #[tokio::test]
    async fn get_json_surfaces_json_parse_errors() {
        let (url, server) = serve_once("200 OK", "not-json");
        let err = get_json::<Vec<ApiEntry>>(&url).await.unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.starts_with("huggingface json parse:")
        ));
    }

    #[tokio::test]
    async fn get_json_rejects_oversized_body() {
        let body = "x".repeat(MAX_BYTES + 1);
        let (url, server) = serve_once("200 OK", &body);
        let err = get_json::<Vec<ApiEntry>>(&url).await.unwrap_err();
        server.join().unwrap();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("huggingface response too large")
        ));
    }

    #[tokio::test]
    async fn get_json_surfaces_request_failures() {
        let err = get_json::<Vec<ApiEntry>>("not-a-url").await.unwrap_err();
        assert!(matches!(
            err,
            FetchError::Failed(msg) if msg.contains("huggingface request failed")
        ));
    }

    #[tokio::test]
    async fn render_for_shape_delegates_non_image_shapes_to_render_sync() {
        let body = render_for_shape(
            &[entry("author/repo", 0, 100, None)],
            Kind::Models,
            Shape::Text,
        )
        .await;
        assert!(matches!(body, Body::Text(_)));
    }

    #[tokio::test]
    async fn render_for_shape_builds_image_list_without_avatar_for_idless_entries() {
        let body = render_for_shape(
            &[entry("no_slash", 0, 5, None)],
            Kind::Models,
            Shape::ImageLinkedList,
        )
        .await;
        let Body::ImageLinkedList(d) = body else {
            panic!("expected image_linked_list");
        };
        assert_eq!(d.items.len(), 1);
        assert!(d.items[0].thumbnail_path.is_none());
    }

    #[tokio::test]
    async fn resolve_thumbnails_returns_none_for_authorless_and_empty_inputs() {
        assert!(resolve_thumbnails(&[]).await.is_empty());
        let resolved = resolve_thumbnails(&[entry("no_slash", 0, 0, None)]).await;
        assert_eq!(resolved, vec![None]);
    }
}
