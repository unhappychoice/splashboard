//! `basic_heatmap` — 2D intensity grid authored inline. Right for habit trackers, fixed
//! sample heatmaps, and any small grid the user wants to ship in TOML rather than via the
//! `basic_read_store` file-based variant.

use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, HeatmapData, Payload};
use crate::render::Shape;

use super::common;

const SHAPES: &[Shape] = &[Shape::Heatmap];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "cells",
        type_hint: "2D list of u32, row-major",
        required: false,
        default: Some("[]"),
        description: "Cell intensities. `cells[row][col]` — rows can be ragged but the renderer treats trailing missing cells as zeros.",
    },
    OptionSchema {
        name: "thresholds",
        type_hint: "list of u32",
        required: false,
        default: None,
        description: "Explicit bucket boundaries. When omitted, `grid_heatmap` auto-quartiles from the data.",
    },
    OptionSchema {
        name: "row_labels",
        type_hint: "list of strings",
        required: false,
        default: None,
        description: "One label per row. Renderers display these along the left edge when space allows.",
    },
    OptionSchema {
        name: "col_labels",
        type_hint: "list of strings",
        required: false,
        default: None,
        description: "One label per column. Renderers display these along the top edge when space allows.",
    },
];

pub struct BasicHeatmap;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub cells: Vec<Vec<u32>>,
    #[serde(default)]
    pub thresholds: Option<Vec<u32>>,
    #[serde(default)]
    pub row_labels: Option<Vec<String>>,
    #[serde(default)]
    pub col_labels: Option<Vec<String>>,
}

impl RealtimeFetcher for BasicHeatmap {
    fn name(&self) -> &str {
        "basic_heatmap"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Renders a `Heatmap` payload from inline `[widget.options]`. `cells` is a row-major 2D grid of `u32` intensities; `thresholds` / `row_labels` / `col_labels` are optional. Right for habit trackers, sample / demo grids, or any small fixed heatmap the user wants to keep in TOML."
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        (shape == Shape::Heatmap).then(|| {
            Body::Heatmap(HeatmapData {
                cells: vec![vec![0, 1, 2], vec![1, 3, 1], vec![2, 2, 4]],
                thresholds: None,
                row_labels: None,
                col_labels: None,
            })
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        common::bare(Body::Heatmap(HeatmapData {
            cells: opts.cells,
            thresholds: opts.thresholds,
            row_labels: opts.row_labels,
            col_labels: opts.col_labels,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compute(toml_src: &str) -> Payload {
        BasicHeatmap.compute(&FetchContext {
            options: Some(toml::from_str(toml_src).unwrap()),
            ..Default::default()
        })
    }

    #[test]
    fn contract() {
        assert_eq!(BasicHeatmap.name(), "basic_heatmap");
        assert_eq!(BasicHeatmap.shapes(), &[Shape::Heatmap]);
    }

    #[test]
    fn emits_grid_with_optional_labels_and_thresholds() {
        let p = compute(
            r#"
            cells = [[0, 1, 2], [1, 3, 1]]
            thresholds = [1, 2, 3]
            row_labels = ["Mon", "Tue"]
            col_labels = ["W1", "W2", "W3"]
            "#,
        );
        let Body::Heatmap(d) = p.body else {
            panic!("expected Heatmap");
        };
        assert_eq!(d.cells, vec![vec![0, 1, 2], vec![1, 3, 1]]);
        assert_eq!(d.thresholds, Some(vec![1, 2, 3]));
        assert_eq!(d.row_labels, Some(vec!["Mon".into(), "Tue".into()]));
        assert_eq!(
            d.col_labels,
            Some(vec!["W1".into(), "W2".into(), "W3".into()])
        );
    }

    #[test]
    fn no_options_yields_empty_grid() {
        let p = BasicHeatmap.compute(&FetchContext::default());
        let Body::Heatmap(d) = p.body else {
            panic!("expected Heatmap");
        };
        assert!(d.cells.is_empty());
    }
}
