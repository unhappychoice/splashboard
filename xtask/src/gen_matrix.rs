//! Generate fetcher / renderer reference Markdown from the live registries.
//!
//! Layout:
//! - `<out>/matrix.md` — overview: fetcher index, renderer index, shape×renderer matrix.
//! - `<out>/fetchers/<family>/<name>.md` for families with 2+ members (e.g. `clock_*`,
//!   `git_*`), else `<out>/fetchers/<name>.md` for lone-wolf entries (`static`,
//!   `read_store`, `disk_usage`, …). Same split for renderers. Family = the segment before
//!   the first underscore, so `clock_ratio` joins `clock` in the `clock/` subdir and
//!   Starlight's `autogenerate` surfaces it as a collapsible sidebar group.
//!
//! The whole point is that these pages can't drift from code: anyone adding a fetcher or
//! renderer re-runs `cargo xtask gen-matrix` and the catalog updates mechanically.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use splashboard::fetcher::{RegisteredFetcher, Registry as FetcherRegistry, Safety};
use splashboard::options::OptionSchema;
use splashboard::render::{Registry as RenderRegistry, Renderer, Shape};
use splashboard::theme::ColorKey;

use crate::snapshots;

const ALL_SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::Entries,
    Shape::Ratio,
    Shape::NumberSeries,
    Shape::PointSeries,
    Shape::Bars,
    Shape::Image,
    Shape::Calendar,
    Shape::Heatmap,
    Shape::Badge,
    Shape::Timeline,
];

pub fn run(out: &Path) -> Result<()> {
    let fetchers = FetcherRegistry::with_builtins();
    let renderers = RenderRegistry::with_builtins();
    let fetcher_layout = FamilyLayout::build(fetchers.sorted().iter().map(|f| f.name()));
    let renderer_layout = FamilyLayout::build(renderers.sorted().iter().map(|r| r.name()));
    write_file(
        &out.join("matrix.md"),
        &overview(&fetchers, &renderers, &fetcher_layout, &renderer_layout),
    )?;
    for f in fetchers.sorted() {
        let page = fetcher_page(&f, &renderers, &fetcher_layout, &renderer_layout);
        write_file(
            &out.join(fetcher_layout.rel_path("fetchers", f.name())),
            &page,
        )?;
    }
    for r in renderers.sorted() {
        let page = renderer_page(&r, &fetchers, &renderer_layout, &fetcher_layout);
        write_file(
            &out.join(renderer_layout.rel_path("renderers", r.name())),
            &page,
        )?;
    }
    Ok(())
}

/// Decides whether each catalog entry lives inside a family subdir (`<kind>/<family>/<name>.md`)
/// or flat at the root (`<kind>/<name>.md`). A family (everything before the first `_`, or the
/// whole name if there's no `_`) only gets its own subdir when 2+ entries share it — otherwise
/// a lone-wolf entry like `static` or `media_image` would be forced into a one-item folder,
/// which Starlight renders as a group of one and reads as clutter.
pub struct FamilyLayout {
    grouped_families: HashMap<String, bool>,
}

impl FamilyLayout {
    fn build<'a, I: IntoIterator<Item = &'a str>>(names: I) -> Self {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for name in names {
            *counts.entry(family_of(name).to_string()).or_insert(0) += 1;
        }
        let grouped_families = counts.into_iter().map(|(fam, n)| (fam, n >= 2)).collect();
        Self { grouped_families }
    }

    fn is_grouped(&self, name: &str) -> bool {
        self.grouped_families
            .get(family_of(name))
            .copied()
            .unwrap_or(false)
    }

    /// Path relative to the reference root (e.g. `fetchers/clock/clock_ratio.md`).
    fn rel_path(&self, kind: &str, name: &str) -> String {
        if self.is_grouped(name) {
            format!("{kind}/{}/{name}.md", family_of(name))
        } else {
            format!("{kind}/{name}.md")
        }
    }

    /// Link from a page living in `from_kind`/<…>/<from_name>.md to a page in
    /// `to_kind`/<…>/<to_name>.md. The source page's depth (flat vs grouped) decides how many
    /// `../` hops are needed to climb back to the reference root.
    fn cross_link(
        &self,
        from_name: &str,
        to_layout: &FamilyLayout,
        to_kind: &str,
        to_name: &str,
    ) -> String {
        let up = if self.is_grouped(from_name) {
            "../../"
        } else {
            "../"
        };
        format!("{up}{}", to_layout.rel_path(to_kind, to_name))
    }
}

fn family_of(name: &str) -> &str {
    name.split_once('_').map(|(f, _)| f).unwrap_or(name)
}

fn write_file(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir for {}", path.display()))?;
    }
    fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn overview(
    fetchers: &FetcherRegistry,
    renderers: &RenderRegistry,
    fetcher_layout: &FamilyLayout,
    renderer_layout: &FamilyLayout,
) -> String {
    let mut out = String::new();
    push_frontmatter(
        &mut out,
        "Reference",
        "Fetcher index, renderer index, and the shape × renderer compatibility matrix.",
    );
    out.push_str(
        "Generated by `cargo xtask` from the built-in registries. Do not edit by hand.\n\n",
    );
    overview_fetcher_table(&mut out, fetchers, fetcher_layout);
    overview_renderer_table(&mut out, renderers, renderer_layout);
    overview_shape_matrix(&mut out, renderers);
    out
}

fn overview_fetcher_table(out: &mut String, fetchers: &FetcherRegistry, layout: &FamilyLayout) {
    out.push_str("## Fetchers\n\n");
    out.push_str("| Name | Kind | Safety | Shapes |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for f in fetchers.sorted() {
        let _ = writeln!(
            out,
            "| [`{name}`]({link}) | {kind} | {safety} | {shapes} |",
            name = f.name(),
            link = layout.rel_path("fetchers", f.name()),
            kind = kind_label(&f),
            safety = safety_label(f.safety()),
            shapes = shapes_list(&f.shapes()),
        );
    }
    out.push('\n');
}

fn overview_renderer_table(out: &mut String, renderers: &RenderRegistry, layout: &FamilyLayout) {
    out.push_str("## Renderers\n\n");
    out.push_str("| Name | Accepts | Animates |\n");
    out.push_str("| --- | --- | --- |\n");
    for r in renderers.sorted() {
        let _ = writeln!(
            out,
            "| [`{name}`]({link}) | {accepts} | {animates} |",
            name = r.name(),
            link = layout.rel_path("renderers", r.name()),
            accepts = shapes_list(r.accepts()),
            animates = if r.animates() { "yes" } else { "no" },
        );
    }
    out.push('\n');
}

fn overview_shape_matrix(out: &mut String, renderers: &RenderRegistry) {
    out.push_str("## Shape × Renderer\n\n");
    let sorted = renderers.sorted();
    out.push_str("| Shape \\ Renderer |");
    for r in &sorted {
        let _ = write!(out, " `{}` |", r.name());
    }
    out.push('\n');
    out.push('|');
    for _ in 0..=sorted.len() {
        out.push_str(" --- |");
    }
    out.push('\n');
    for shape in ALL_SHAPES {
        let _ = write!(out, "| `{}` |", shape.as_str());
        for r in &sorted {
            let cell = if r.accepts().contains(shape) {
                "✓"
            } else {
                ""
            };
            let _ = write!(out, " {cell} |");
        }
        out.push('\n');
    }
    out.push('\n');
}

fn fetcher_page(
    f: &RegisteredFetcher,
    renderers: &RenderRegistry,
    fetcher_layout: &FamilyLayout,
    renderer_layout: &FamilyLayout,
) -> String {
    let mut out = String::new();
    let description = format!(
        "Fetcher `{}` — {} ({}). Emits {}.",
        f.name(),
        kind_label(f),
        safety_label(f.safety()),
        shapes_list(&f.shapes()),
    );
    push_frontmatter(&mut out, f.name(), &description);
    let _ = writeln!(out, "- Kind: {}", kind_label(f));
    let _ = writeln!(out, "- Safety: {}", safety_label(f.safety()));
    let _ = writeln!(out, "- Shapes: {}\n", shapes_list(&f.shapes()));
    render_option_section(
        &mut out,
        f.option_schemas(),
        "Options",
        option_fallback_fetcher(),
    );
    render_compatible_renderers(
        &mut out,
        f.name(),
        &f.shapes(),
        renderers,
        fetcher_layout,
        renderer_layout,
    );
    render_previews(&mut out, f, renderers);
    out
}

fn render_previews(out: &mut String, fetcher: &RegisteredFetcher, renderers: &RenderRegistry) {
    let previews = snapshots::previews_for(fetcher, renderers);
    if previews.is_empty() {
        return;
    }
    out.push_str("## Preview\n\n");
    for p in previews {
        let _ = writeln!(out, "### `{}` via `{}`\n", p.shape.as_str(), p.renderer);
        out.push_str(&p.html);
        out.push_str("\n\n");
        out.push_str("```toml\n");
        out.push_str(&p.config_snippet);
        out.push_str("\n```\n\n");
    }
}

fn renderer_page(
    r: &Arc<dyn Renderer>,
    fetchers: &FetcherRegistry,
    renderer_layout: &FamilyLayout,
    fetcher_layout: &FamilyLayout,
) -> String {
    let mut out = String::new();
    let description = format!(
        "Renderer `{}` — accepts {}.",
        r.name(),
        shapes_list(r.accepts()),
    );
    push_frontmatter(&mut out, r.name(), &description);
    let _ = writeln!(out, "- Accepts: {}", shapes_list(r.accepts()));
    let _ = writeln!(
        out,
        "- Animates: {}\n",
        if r.animates() { "yes" } else { "no" }
    );
    render_option_section(
        &mut out,
        r.option_schemas(),
        "Options",
        option_fallback_renderer(),
    );
    render_color_keys_section(&mut out, r.color_keys());
    render_compatible_fetchers(
        &mut out,
        r.name(),
        r.accepts(),
        fetchers,
        renderer_layout,
        fetcher_layout,
    );
    out
}

fn render_color_keys_section(out: &mut String, keys: &[ColorKey]) {
    out.push_str("## Theme tokens\n\n");
    if keys.is_empty() {
        out.push_str("_This renderer does not read any `[theme]` tokens._\n\n");
        return;
    }
    out.push_str("Overridable via the `[theme]` section of `config.toml`.\n\n");
    out.push_str("| Key | Description |\n");
    out.push_str("| --- | --- |\n");
    for k in keys {
        let _ = writeln!(
            out,
            "| `{name}` | {desc} |",
            name = k.name,
            desc = escape_pipe(k.description),
        );
    }
    out.push('\n');
}

/// Starlight uses `title` from frontmatter; authoring an H1 in the body would duplicate.
fn push_frontmatter(out: &mut String, title: &str, description: &str) {
    out.push_str("---\n");
    let _ = writeln!(out, "title: {}", yaml_string(title));
    let _ = writeln!(out, "description: {}", yaml_string(description));
    out.push_str("---\n\n");
}

/// YAML scalar that survives colons, backticks, and the other metacharacters we actually
/// generate. Double-quoted with backslash escapes is the lowest-footgun choice.
fn yaml_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn render_option_section(
    out: &mut String,
    schemas: &[OptionSchema],
    heading: &str,
    fallback: &str,
) {
    let _ = writeln!(out, "## {heading}\n");
    if schemas.is_empty() {
        let _ = writeln!(out, "{fallback}\n");
        return;
    }
    out.push_str("| Option | Type | Required | Default | Description |\n");
    out.push_str("| --- | --- | --- | --- | --- |\n");
    for s in schemas {
        let required = if s.required { "yes" } else { "no" };
        let default = s.default.unwrap_or("—");
        let _ = writeln!(
            out,
            "| `{name}` | {ty} | {required} | {default} | {desc} |",
            name = s.name,
            ty = escape_pipe(s.type_hint),
            default = escape_pipe(default),
            desc = escape_pipe(s.description),
        );
    }
    out.push('\n');
}

fn option_fallback_fetcher() -> &'static str {
    "_No options._"
}

fn option_fallback_renderer() -> &'static str {
    "_No options beyond the shared `RenderOptions` defaults._"
}

fn render_compatible_renderers(
    out: &mut String,
    from_name: &str,
    shapes: &[Shape],
    renderers: &RenderRegistry,
    fetcher_layout: &FamilyLayout,
    renderer_layout: &FamilyLayout,
) {
    out.push_str("## Compatible renderers\n\n");
    out.push_str("| Shape | Renderers |\n");
    out.push_str("| --- | --- |\n");
    let sorted = renderers.sorted();
    for shape in shapes {
        let matches: Vec<String> = sorted
            .iter()
            .filter(|r| r.accepts().contains(shape))
            .map(|r| {
                format!(
                    "[`{name}`]({link})",
                    name = r.name(),
                    link = fetcher_layout.cross_link(
                        from_name,
                        renderer_layout,
                        "renderers",
                        r.name()
                    ),
                )
            })
            .collect();
        let cell = if matches.is_empty() {
            "_none_".into()
        } else {
            matches.join(", ")
        };
        let _ = writeln!(out, "| `{}` | {} |", shape.as_str(), cell);
    }
    out.push('\n');
}

fn render_compatible_fetchers(
    out: &mut String,
    from_name: &str,
    shapes: &[Shape],
    fetchers: &FetcherRegistry,
    renderer_layout: &FamilyLayout,
    fetcher_layout: &FamilyLayout,
) {
    out.push_str("## Compatible fetchers\n\n");
    out.push_str("| Shape | Fetchers |\n");
    out.push_str("| --- | --- |\n");
    let sorted = fetchers.sorted();
    for shape in shapes {
        let matches: Vec<String> = sorted
            .iter()
            .filter(|f| f.shapes().contains(shape))
            .map(|f| {
                format!(
                    "[`{name}`]({link})",
                    name = f.name(),
                    link =
                        renderer_layout.cross_link(from_name, fetcher_layout, "fetchers", f.name()),
                )
            })
            .collect();
        let cell = if matches.is_empty() {
            "_none_".into()
        } else {
            matches.join(", ")
        };
        let _ = writeln!(out, "| `{}` | {} |", shape.as_str(), cell);
    }
    out.push('\n');
}

fn kind_label(f: &RegisteredFetcher) -> &'static str {
    match f {
        RegisteredFetcher::Cached(_) => "cached",
        RegisteredFetcher::Realtime(_) => "realtime",
    }
}

fn safety_label(s: Safety) -> &'static str {
    match s {
        Safety::Safe => "Safe",
        Safety::Network => "Network",
        Safety::Exec => "Exec",
    }
}

fn shapes_list(shapes: &[Shape]) -> String {
    shapes
        .iter()
        .map(|s| format!("`{}`", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Escape pipe characters inside Markdown table cells. Without this, a type hint like
/// `"\"full\" | \"quadrant\""` breaks the row.
fn escape_pipe(s: &str) -> String {
    s.replace('|', "\\|")
}
