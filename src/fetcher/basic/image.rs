//! `basic_image` — render a file from disk as the splash image. The escape hatch for repo logos
//! and per-directory branding without writing a fetcher.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, ImageData, Payload};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Image];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "path",
    type_hint: "string (file path)",
    required: true,
    default: None,
    description: "Absolute or `~`-expanded path to a PNG / JPEG / GIF / WebP. Loaded by `media_image` at draw time; this fetcher only passes the string through.",
}];

pub struct BasicImage;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub path: Option<String>,
}

impl RealtimeFetcher for BasicImage {
    fn name(&self) -> &str {
        "basic_image"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Emits a user-configured file path as an `Image` payload. Right for repo logos, per-directory branding, or any static image — the renderer (`media_image`) is what actually loads the bytes. Path expansion is deferred to the renderer so this fetcher is pure and infallible at compute time."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Image).then(|| {
            Body::Image(ImageData {
                path: "/path/to/logo.png".into(),
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        match opts.path.filter(|p| !p.trim().is_empty()) {
            Some(path) => common::bare(Body::Image(ImageData { path })),
            None => common::placeholder("basic_image: `path` is required"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract() {
        let f = BasicImage;
        assert_eq!(f.name(), "basic_image");
        assert_eq!(f.safety(), Safety::Safe);
        assert_eq!(f.shapes(), &[Shape::Image]);
    }

    #[test]
    fn emits_image_body_with_supplied_path() {
        let p = BasicImage.compute(&FetchContext {
            options: Some(toml::from_str(r#"path = "/tmp/logo.png""#).unwrap()),
            ..Default::default()
        });
        assert_eq!(
            p.body,
            Body::Image(ImageData {
                path: "/tmp/logo.png".into(),
            })
        );
    }

    #[test]
    fn missing_path_renders_placeholder() {
        let p = BasicImage.compute(&FetchContext::default());
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("`path` is required"));
    }

    #[test]
    fn blank_path_renders_placeholder() {
        let p = BasicImage.compute(&FetchContext {
            options: Some(toml::from_str(r#"path = "   ""#).unwrap()),
            ..Default::default()
        });
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("`path` is required"));
    }

    #[test]
    fn metadata_methods_have_content() {
        let f = BasicImage;
        assert!(!f.description().is_empty());
        assert_eq!(
            f.option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["path"]
        );
    }

    #[test]
    fn sample_body_matches_declared_shape_only() {
        let f = BasicImage;
        assert!(matches!(f.sample_body(Shape::Image), Some(Body::Image(_))));
        assert!(f.sample_body(Shape::Text).is_none());
    }

    #[test]
    fn invalid_options_render_placeholder() {
        let p = BasicImage.compute(&FetchContext {
            options: Some(toml::from_str(r#"path = 42"#).unwrap()),
            ..Default::default()
        });
        let Body::TextBlock(d) = p.body else {
            panic!("expected placeholder");
        };
        assert!(d.lines[0].contains("invalid options"));
    }
}
