//! Workspace tooling for splashboard. Run via `cargo xtask`.
//!
//! Reads the built-in fetcher / renderer registries and writes Markdown reference pages (one
//! per fetcher, one per renderer, plus an overview). Also renders full dashboard TOMLs to
//! inline HTML using each fetcher's `sample_body`, so the docs site can show real splash
//! output without ever touching the network.

use std::path::{Path, PathBuf};
use std::{fs, io::Write};

use anyhow::{Context, Result};
use clap::Parser;

mod dashboard_snapshot;
mod gen_matrix;
mod html_snapshot;
mod snapshots;

#[derive(Parser)]
#[command(
    version,
    about = "Generate splashboard's fetcher/renderer reference pages"
)]
struct Cli {
    /// Destination directory. Writes `<out>/matrix.md`, `<out>/fetchers/<name>.md`,
    /// `<out>/renderers/<name>.md`. Default lands inside the Starlight content tree so
    /// `astro build` picks the pages up without further wiring.
    #[arg(long, default_value = "docs-site/src/content/docs/reference")]
    out: PathBuf,

    /// Destination directory for rendered dashboard HTML snippets, imported via `?raw` by
    /// `index.mdx` and other landing surfaces. One file per entry in `DASHBOARD_SNAPSHOTS`.
    #[arg(long, default_value = "docs-site/src/assets/rendered")]
    rendered_out: PathBuf,
}

/// Each entry renders `source_config` at `(width, height)` and writes the HTML to `output_name`
/// under `--rendered-out`. Sizes are picked per-config so the whole layout lands without clipping.
const DASHBOARD_SNAPSHOTS: &[(&str, &str, u16, u16)] = &[(
    ".splashboard/dashboard.toml",
    "project_github_activity.html",
    120,
    42,
)];

fn main() -> Result<()> {
    let cli = Cli::parse();
    gen_matrix::run(&cli.out)?;
    render_dashboards(&cli.rendered_out)?;
    Ok(())
}

fn render_dashboards(out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    for (config, output_name, width, height) in DASHBOARD_SNAPSHOTS {
        let html = dashboard_snapshot::render_config_html(Path::new(config), *width, *height)?;
        let dest = out_dir.join(output_name);
        let mut f =
            fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        f.write_all(html.as_bytes())
            .with_context(|| format!("write {}", dest.display()))?;
        println!("wrote {}", dest.display());
    }
    Ok(())
}
