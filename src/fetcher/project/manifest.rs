//! `project_manifest` — reads version, description, name, and license from the nearest
//! project manifest file (Cargo.toml › package.json › pyproject.toml › go.mod › composer.json)
//! walking up from the process CWD. `Text` emits the version string; `Entries` delivers all
//! available fields as key/value rows; `TextBlock` lists them as human-readable lines;
//! `MarkdownTextBlock` formats name / description / version / license as a styled block;
//! `Badge` shows the version with a tone derived from license permissiveness.

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::detect::{ManifestData, cwd_cache_component, detect_manifest};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::Entries,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Badge,
];

pub struct ProjectManifest;

#[async_trait]
impl Fetcher for ProjectManifest {
    fn name(&self) -> &str {
        "project_manifest"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Reads project metadata from the nearest manifest file found by walking up from the \
         process CWD — Cargo.toml, package.json, pyproject.toml, go.mod, or composer.json. \
         `Text` emits the version string (useful as a hero subtitle); `Entries` delivers \
         name / version / description / license as key/value rows; `TextBlock` lists the \
         same fields as human-readable lines; `MarkdownTextBlock` formats them as a styled \
         block; `Badge` shows the version with a tone derived from license permissiveness \
         (permissive → ok, copyleft → warn, unknown → warn)."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 5
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn default_shape(&self) -> Shape {
        Shape::Entries
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let shape = ctx.shape.map(|s| s.as_str()).unwrap_or("default");
        let raw = format!("{}|{}|{}", self.name(), shape, cwd_cache_component());
        let digest = Sha256::digest(raw.as_bytes());
        let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        format!("{}-{hex}", self.name())
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("1.4.2"),
            Shape::Entries => samples::entries(&[
                ("name", "splashboard"),
                ("version", "1.4.2"),
                ("description", "Terminal splash dashboard"),
                ("license", "MIT"),
            ]),
            Shape::TextBlock => {
                samples::text_block(&["splashboard 1.4.2", "Terminal splash dashboard", "MIT"])
            }
            Shape::MarkdownTextBlock => {
                samples::markdown("# splashboard\n\n_Terminal splash dashboard_\n\n**1.4.2** · MIT")
            }
            Shape::Badge => samples::badge(Status::Ok, "v1.4.2"),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let data = detect_manifest().unwrap_or_default();
        let shape = ctx.shape.unwrap_or(Shape::Entries);
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body: render_body(&data, shape),
        })
    }
}

fn render_body(data: &ManifestData, shape: Shape) -> Body {
    match shape {
        Shape::Text => text_body(data),
        Shape::TextBlock => text_block_body(data),
        Shape::MarkdownTextBlock => markdown_body(data),
        Shape::Badge => badge_body(data),
        _ => entries_body(data),
    }
}

fn text_body(data: &ManifestData) -> Body {
    Body::Text(TextData {
        value: data.version.clone().unwrap_or_default(),
    })
}

fn text_block_body(data: &ManifestData) -> Body {
    let mut lines = Vec::new();
    if let Some(name) = &data.name {
        let header = match &data.version {
            Some(v) => format!("{name} {v}"),
            None => name.clone(),
        };
        lines.push(header);
    }
    if let Some(desc) = &data.description {
        lines.push(desc.clone());
    }
    if let Some(lic) = &data.license {
        lines.push(lic.clone());
    }
    Body::TextBlock(TextBlockData { lines })
}

fn markdown_body(data: &ManifestData) -> Body {
    let mut sections: Vec<String> = Vec::new();
    if let Some(name) = &data.name {
        sections.push(format!("# {name}"));
    }
    if let Some(desc) = &data.description {
        sections.push(format!("_{desc}_"));
    }
    let footer = match (&data.version, &data.license) {
        (Some(v), Some(l)) => Some(format!("**{v}** · {l}")),
        (Some(v), None) => Some(format!("**{v}**")),
        (None, Some(l)) => Some(l.clone()),
        (None, None) => None,
    };
    if let Some(f) = footer {
        sections.push(f);
    }
    Body::MarkdownTextBlock(MarkdownTextBlockData {
        value: sections.join("\n\n"),
    })
}

fn badge_body(data: &ManifestData) -> Body {
    match (&data.version, &data.name) {
        (Some(v), _) => Body::Badge(BadgeData {
            status: license_status(data.license.as_deref()),
            label: format!("v{v}"),
        }),
        (None, Some(n)) => Body::Badge(BadgeData {
            status: Status::Warn,
            label: n.clone(),
        }),
        (None, None) => Body::Badge(BadgeData {
            status: Status::Warn,
            label: String::new(),
        }),
    }
}

fn license_status(license: Option<&str>) -> Status {
    match license {
        None => Status::Warn,
        Some(s) => {
            let lower = s.to_ascii_lowercase();
            if lower.contains("agpl") || lower.contains("gpl") {
                Status::Warn
            } else {
                Status::Ok
            }
        }
    }
}

fn entries_body(data: &ManifestData) -> Body {
    let fields: &[(&str, &Option<String>)] = &[
        ("name", &data.name),
        ("version", &data.version),
        ("description", &data.description),
        ("license", &data.license),
    ];
    Body::Entries(EntriesData {
        items: fields
            .iter()
            .filter_map(|(key, val)| {
                val.as_ref().map(|v| Entry {
                    key: key.to_string(),
                    value: Some(v.clone()),
                    status: None,
                })
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::future::Future;

    use tempfile::tempdir;

    use super::*;
    use crate::render::Shape;

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn ctx(shape: Shape) -> FetchContext {
        FetchContext {
            widget_id: "w".into(),
            shape: Some(shape),
            ..Default::default()
        }
    }

    fn manifest() -> ManifestData {
        ManifestData {
            name: Some("myapp".into()),
            version: Some("1.0.0".into()),
            description: Some("A cool app".into()),
            license: Some("MIT".into()),
            ecosystem: "cargo",
        }
    }

    #[test]
    fn fetcher_contract() {
        assert_eq!(ProjectManifest.name(), "project_manifest");
        assert!(matches!(ProjectManifest.safety(), Safety::Safe));
        assert_eq!(ProjectManifest.default_shape(), Shape::Entries);
        assert_eq!(ProjectManifest.shapes(), SHAPES);
        let key_a = ProjectManifest.cache_key(&ctx(Shape::Text));
        let key_b = ProjectManifest.cache_key(&ctx(Shape::Entries));
        assert_ne!(key_a, key_b);
        assert!(key_a.starts_with("project_manifest-"));
    }

    #[test]
    fn samples_cover_all_shapes() {
        SHAPES.iter().copied().for_each(|s| {
            let body = ProjectManifest.sample_body(s).expect("sample body");
            assert_eq!(crate::render::shape_of(&body), s);
        });
        assert!(ProjectManifest.sample_body(Shape::Ratio).is_none());
    }

    #[test]
    fn text_body_returns_version() {
        assert_eq!(
            render_body(&manifest(), Shape::Text),
            Body::Text(TextData {
                value: "1.0.0".into()
            })
        );
    }

    #[test]
    fn text_body_empty_when_no_version() {
        let data = ManifestData::default();
        assert_eq!(
            render_body(&data, Shape::Text),
            Body::Text(TextData {
                value: String::new()
            })
        );
    }

    #[test]
    fn markdown_body_composes_heading_description_footer() {
        let Body::MarkdownTextBlock(md) = render_body(&manifest(), Shape::MarkdownTextBlock) else {
            panic!("expected MarkdownTextBlock");
        };
        assert_eq!(md.value, "# myapp\n\n_A cool app_\n\n**1.0.0** · MIT");
    }

    #[test]
    fn markdown_body_skips_missing_sections() {
        let data = ManifestData {
            name: Some("bare".into()),
            version: None,
            description: None,
            license: None,
            ecosystem: "go",
        };
        let Body::MarkdownTextBlock(md) = render_body(&data, Shape::MarkdownTextBlock) else {
            panic!("expected MarkdownTextBlock");
        };
        assert_eq!(md.value, "# bare");
    }

    #[test]
    fn badge_body_labels_version_with_license_tone() {
        assert_eq!(
            render_body(&manifest(), Shape::Badge),
            Body::Badge(BadgeData {
                status: Status::Ok,
                label: "v1.0.0".into(),
            })
        );
    }

    #[test]
    fn badge_body_warns_on_copyleft_license() {
        let data = ManifestData {
            license: Some("GPL-3.0".into()),
            ..manifest()
        };
        let Body::Badge(b) = render_body(&data, Shape::Badge) else {
            panic!("expected Badge");
        };
        assert_eq!(b.status, Status::Warn);
    }

    #[test]
    fn badge_body_falls_back_to_name_when_version_missing() {
        let data = ManifestData {
            version: None,
            ..manifest()
        };
        let Body::Badge(b) = render_body(&data, Shape::Badge) else {
            panic!("expected Badge");
        };
        assert_eq!(b.label, "myapp");
        assert_eq!(b.status, Status::Warn);
    }

    #[test]
    fn license_status_classifies_permissive_and_copyleft() {
        assert_eq!(license_status(Some("MIT")), Status::Ok);
        assert_eq!(license_status(Some("Apache-2.0")), Status::Ok);
        assert_eq!(license_status(Some("ISC")), Status::Ok);
        assert_eq!(license_status(Some("GPL-3.0")), Status::Warn);
        assert_eq!(license_status(Some("AGPL-3.0-only")), Status::Warn);
        assert_eq!(license_status(None), Status::Warn);
    }

    #[test]
    fn text_block_body_combines_name_and_version() {
        let block = render_body(&manifest(), Shape::TextBlock);
        assert_eq!(
            block,
            Body::TextBlock(TextBlockData {
                lines: vec!["myapp 1.0.0".into(), "A cool app".into(), "MIT".into(),]
            })
        );
    }

    #[test]
    fn text_block_body_skips_missing_fields() {
        let data = ManifestData {
            name: Some("bare".into()),
            version: None,
            description: None,
            license: None,
            ecosystem: "go",
        };
        assert_eq!(
            render_body(&data, Shape::TextBlock),
            Body::TextBlock(TextBlockData {
                lines: vec!["bare".into()]
            })
        );
    }

    #[test]
    fn entries_body_filters_none_values() {
        let data = ManifestData {
            name: Some("myapp".into()),
            version: Some("2.0.0".into()),
            description: None,
            license: None,
            ecosystem: "npm",
        };
        let Body::Entries(entries) = render_body(&data, Shape::Entries) else {
            panic!("expected Entries");
        };
        assert_eq!(entries.items.len(), 2);
        assert_eq!(entries.items[0].key, "name");
        assert_eq!(entries.items[1].key, "version");
    }

    #[test]
    fn fetch_from_splashboard_repo_returns_cargo_entries() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let payload = run_async(ProjectManifest.fetch(&ctx(Shape::Entries))).unwrap();
        let Body::Entries(entries) = payload.body else {
            panic!("expected Entries");
        };
        assert!(entries.items.iter().any(|e| e.key == "name"));
        assert!(
            entries
                .items
                .iter()
                .any(|e| e.key == "version" && e.value.is_some())
        );
    }

    #[test]
    fn fetch_text_from_tmpdir_returns_empty_string() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let payload = run_async(ProjectManifest.fetch(&ctx(Shape::Text))).unwrap();
        std::env::set_current_dir(prev).unwrap();
        let Body::Text(t) = payload.body else {
            panic!("expected Text");
        };
        assert_eq!(t.value, "");
    }

    #[test]
    fn fetch_reads_package_json_in_cwd() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"demo","version":"5.0.0","description":"Demo","license":"Apache-2.0"}"#,
        )
        .unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let payload = run_async(ProjectManifest.fetch(&ctx(Shape::Entries))).unwrap();
        std::env::set_current_dir(prev).unwrap();
        let Body::Entries(e) = payload.body else {
            panic!("expected Entries");
        };
        assert!(
            e.items
                .iter()
                .any(|i| i.key == "name" && i.value.as_deref() == Some("demo"))
        );
        assert!(
            e.items
                .iter()
                .any(|i| i.key == "version" && i.value.as_deref() == Some("5.0.0"))
        );
    }
}
