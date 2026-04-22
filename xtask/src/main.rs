//! Workspace tooling for splashboard. Run via `cargo xtask`.
//!
//! Reads the built-in fetcher / renderer registries and writes Markdown reference pages (one
//! per fetcher, one per renderer, plus an overview). Snapshot / preview generation lives with
//! the Astro docs-site (future work) so HTML production stays a build-time concern of the site,
//! not this tool.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    gen_matrix::run(&cli.out)
}
