//! `basic_entries` — key/value rows authored inline. The TOML twin of `grid_table`'s consumer
//! shape, for project-facts panels (owner / language / license) or any small reference table.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, Status};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "entries",
    type_hint: "list of {key, value?, status?}",
    required: false,
    default: Some("[]"),
    description: "Key/value rows. `status` (ok / warn / error) tints the row in renderers that surface it.",
}];

pub struct BasicEntries;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub entries: Vec<EntryConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryConfig {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub status: Option<Status>,
}

impl RealtimeFetcher for BasicEntries {
    fn name(&self) -> &str {
        "basic_entries"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders an `Entries` payload from inline `[widget.options].entries = [{key, value?, status?}]`. Right for project-facts panels (owner / language / license / docs-link) or any small reference table — the data is stable enough to live in TOML."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Entries).then(|| {
            Body::Entries(EntriesData {
                items: vec![
                    Entry {
                        key: "Owner".into(),
                        value: Some("team-alpha".into()),
                        status: None,
                    },
                    Entry {
                        key: "Lang".into(),
                        value: Some("Rust".into()),
                        status: None,
                    },
                ],
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        common::bare(Body::Entries(EntriesData {
            items: opts
                .entries
                .into_iter()
                .map(|e| Entry {
                    key: e.key,
                    value: e.value,
                    status: e.status,
                })
                .collect(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compute(toml_src: &str) -> Payload {
        BasicEntries.compute(&FetchContext {
            options: Some(toml::from_str(toml_src).unwrap()),
            ..Default::default()
        })
    }

    #[test]
    fn contract() {
        assert_eq!(BasicEntries.name(), "basic_entries");
        assert_eq!(BasicEntries.shapes(), &[Shape::Entries]);
    }

    #[test]
    fn emits_entries_preserving_input_order() {
        let p = compute(
            r#"
            [[entries]]
            key = "Owner"
            value = "team-alpha"
            [[entries]]
            key = "Lang"
            value = "Rust"
            status = "ok"
            "#,
        );
        let Body::Entries(d) = p.body else {
            panic!("expected Entries");
        };
        assert_eq!(d.items.len(), 2);
        assert_eq!(d.items[0].key, "Owner");
        assert_eq!(d.items[0].value.as_deref(), Some("team-alpha"));
        assert_eq!(d.items[1].status, Some(Status::Ok));
    }

    #[test]
    fn value_is_optional() {
        let p = compute(
            r#"
            [[entries]]
            key = "WIP"
            "#,
        );
        let Body::Entries(d) = p.body else {
            panic!("expected Entries");
        };
        assert_eq!(d.items[0].key, "WIP");
        assert!(d.items[0].value.is_none());
    }

    #[test]
    fn no_entries_yields_empty_payload() {
        let p = BasicEntries.compute(&FetchContext::default());
        let Body::Entries(d) = p.body else {
            panic!("expected Entries");
        };
        assert!(d.items.is_empty());
    }

    #[test]
    fn metadata_methods_have_content() {
        let f = BasicEntries;
        assert_eq!(f.safety(), Safety::Safe);
        assert!(!f.description().is_empty());
        assert_eq!(
            f.option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["entries"]
        );
    }

    #[test]
    fn sample_body_matches_declared_shape_only() {
        let f = BasicEntries;
        assert!(matches!(
            f.sample_body(Shape::Entries),
            Some(Body::Entries(_))
        ));
        assert!(f.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn invalid_options_render_placeholder() {
        let p = compute(r#"entries = "not-a-list""#);
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("invalid options"));
    }
}
