//! TTY-formatted views over the fetcher / renderer catalog. Same data the `xtask` crate
//! renders as Markdown for the docs site, reshaped for interactive terminal browsing:
//! column-aligned plain text, compact enough for 80-column windows, dependency-free.

use std::fmt::Write as _;

use crate::fetcher::{RegisteredFetcher, Registry as FetcherRegistry, Safety};
use crate::options::OptionSchema;
use crate::render::{Registry as RenderRegistry, Renderer, Shape};

pub fn overview(fetchers: &FetcherRegistry, renderers: &RenderRegistry) -> String {
    let mut out = String::new();
    out.push_str(&fetcher_list(fetchers));
    out.push('\n');
    out.push_str(&renderer_list(renderers));
    out.push('\n');
    out.push_str(
        "Run `splashboard catalog fetcher <NAME>` or `catalog renderer <NAME>` for details.\n",
    );
    out
}

pub fn fetcher_list(fetchers: &FetcherRegistry) -> String {
    let rows: Vec<Vec<String>> = std::iter::once(header(&["NAME", "KIND", "SAFETY", "SHAPES"]))
        .chain(fetchers.sorted().iter().map(fetcher_row))
        .collect();
    format!(
        "Fetchers ({}):\n{}\n",
        fetchers.sorted().len(),
        format_table(&rows)
    )
}

pub fn fetcher_detail(
    name: &str,
    fetchers: &FetcherRegistry,
    renderers: &RenderRegistry,
) -> Result<String, String> {
    let f = fetchers.get(name).ok_or_else(|| {
        unknown_error(
            "fetcher",
            name,
            fetchers.sorted().iter().map(|f| f.name().to_string()),
        )
    })?;
    let mut out = format!("Fetcher `{}`\n", f.name());
    let _ = writeln!(out, "  Kind:    {}", kind_label(&f));
    let _ = writeln!(out, "  Safety:  {}", safety_label(f.safety()));
    let _ = writeln!(out, "  Shapes:  {}", shapes_list(&f.shapes()));
    out.push_str(&options_section(f.option_schemas()));
    out.push_str(&compatible_section(
        "Compatible renderers",
        &f.shapes(),
        |shape| {
            renderers
                .sorted()
                .into_iter()
                .filter(|r| r.accepts().contains(shape))
                .map(|r| r.name().to_string())
                .collect()
        },
    ));
    Ok(out)
}

pub fn renderer_list(renderers: &RenderRegistry) -> String {
    let rows: Vec<Vec<String>> = std::iter::once(header(&["NAME", "ACCEPTS", "ANIMATES"]))
        .chain(renderers.sorted().iter().map(renderer_row))
        .collect();
    format!(
        "Renderers ({}):\n{}\n",
        renderers.sorted().len(),
        format_table(&rows)
    )
}

pub fn renderer_detail(
    name: &str,
    renderers: &RenderRegistry,
    fetchers: &FetcherRegistry,
) -> Result<String, String> {
    let r = renderers.get(name).ok_or_else(|| {
        unknown_error(
            "renderer",
            name,
            renderers.sorted().iter().map(|r| r.name().to_string()),
        )
    })?;
    let mut out = format!("Renderer `{}`\n", r.name());
    let _ = writeln!(out, "  Accepts:  {}", shapes_list(r.accepts()));
    let _ = writeln!(out, "  Animates: {}", yes_no(r.animates()));
    out.push_str(&options_section(r.option_schemas()));
    out.push_str(&compatible_section(
        "Compatible fetchers",
        r.accepts(),
        |shape| {
            fetchers
                .sorted()
                .into_iter()
                .filter(|f| f.shapes().contains(shape))
                .map(|f| f.name().to_string())
                .collect()
        },
    ));
    Ok(out)
}

fn fetcher_row(f: &RegisteredFetcher) -> Vec<String> {
    vec![
        f.name().to_string(),
        kind_label(f).to_string(),
        safety_label(f.safety()).to_string(),
        shapes_list(&f.shapes()),
    ]
}

fn renderer_row(r: &std::sync::Arc<dyn Renderer>) -> Vec<String> {
    vec![
        r.name().to_string(),
        shapes_list(r.accepts()),
        yes_no(r.animates()).to_string(),
    ]
}

fn options_section(schemas: &[OptionSchema]) -> String {
    if schemas.is_empty() {
        return "\nOptions: (none)\n".into();
    }
    let rows: Vec<Vec<String>> = std::iter::once(header(&["NAME", "TYPE", "REQUIRED", "DEFAULT"]))
        .chain(schemas.iter().map(option_row))
        .collect();
    let widths = column_widths(&rows);
    let body: String = rows
        .iter()
        .skip(1)
        .zip(schemas)
        .map(|(row, s)| {
            format!(
                "{}\n      {}\n",
                format_row(row, &widths),
                sanitize(s.description)
            )
        })
        .collect();
    format!("\nOptions:\n{}\n{}", format_row(&rows[0], &widths), body)
}

fn option_row(s: &OptionSchema) -> Vec<String> {
    vec![
        s.name.to_string(),
        sanitize(s.type_hint),
        (if s.required { "required" } else { "optional" }).to_string(),
        sanitize(s.default.unwrap_or("-")),
    ]
}

fn compatible_section(
    heading: &str,
    shapes: &[Shape],
    lookup: impl Fn(&Shape) -> Vec<String>,
) -> String {
    let mut out = format!("\n{heading}:\n");
    for shape in shapes {
        let names = lookup(shape);
        let cell = if names.is_empty() {
            "(none)".into()
        } else {
            names.join(", ")
        };
        let _ = writeln!(out, "  {} -> {}", shape.as_str(), cell);
    }
    out
}

fn format_table(rows: &[Vec<String>]) -> String {
    let widths = column_widths(rows);
    rows.iter()
        .map(|r| format_row(r, &widths))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_row(cells: &[String], widths: &[usize]) -> String {
    let last = cells.len().saturating_sub(1);
    let padded: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if i == last {
                c.clone()
            } else {
                format!("{:<width$}  ", c, width = widths[i])
            }
        })
        .collect();
    format!("  {}", padded.concat())
}

fn column_widths(rows: &[Vec<String>]) -> Vec<usize> {
    let ncols = rows.first().map(|r| r.len()).unwrap_or(0);
    (0..ncols)
        .map(|c| {
            rows.iter()
                .map(|r| r.get(c).map(|s| s.chars().count()).unwrap_or(0))
                .max()
                .unwrap_or(0)
        })
        .collect()
}

fn header(labels: &[&str]) -> Vec<String> {
    labels.iter().map(|s| (*s).to_string()).collect()
}

fn unknown_error(kind: &str, name: &str, available: impl Iterator<Item = String>) -> String {
    let joined: Vec<String> = available.collect();
    format!("unknown {kind}: {name}\n\navailable: {}", joined.join(", "))
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
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Scrubs control characters out of static schema strings before they hit the terminal. The
/// inputs are `&'static str` from code, so this is defense in depth — mirrors the hardening in
/// `main.rs::sanitize_for_display` for the trust prompt.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_lists_both_sections() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let out = overview(&fetchers, &renderers);
        assert!(out.contains("Fetchers ("));
        assert!(out.contains("Renderers ("));
        assert!(out.contains("clock"));
        assert!(out.contains("text_ascii"));
    }

    #[test]
    fn fetcher_detail_renders_builtin() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let out = fetcher_detail("clock", &fetchers, &renderers).unwrap();
        assert!(out.contains("Fetcher `clock`"));
        assert!(out.contains("Kind:"));
        assert!(out.contains("Safety:"));
        assert!(out.contains("Compatible renderers"));
    }

    #[test]
    fn fetcher_detail_errors_on_unknown() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let err = fetcher_detail("nope", &fetchers, &renderers).unwrap_err();
        assert!(err.starts_with("unknown fetcher: nope"));
        assert!(err.contains("available:"));
    }

    #[test]
    fn renderer_detail_renders_builtin() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let out = renderer_detail("text_ascii", &renderers, &fetchers).unwrap();
        assert!(out.contains("Renderer `text_ascii`"));
        assert!(out.contains("Accepts:"));
        assert!(out.contains("Animates:"));
        assert!(out.contains("Compatible fetchers"));
    }

    #[test]
    fn renderer_detail_errors_on_unknown() {
        let fetchers = FetcherRegistry::with_builtins();
        let renderers = RenderRegistry::with_builtins();
        let err = renderer_detail("nope", &renderers, &fetchers).unwrap_err();
        assert!(err.starts_with("unknown renderer: nope"));
    }

    #[test]
    fn fetcher_list_includes_header_and_entries() {
        let fetchers = FetcherRegistry::with_builtins();
        let out = fetcher_list(&fetchers);
        assert!(out.contains("NAME"));
        assert!(out.contains("KIND"));
        assert!(out.contains("SAFETY"));
        assert!(out.contains("SHAPES"));
        assert!(out.contains("clock"));
    }

    #[test]
    fn renderer_list_includes_header_and_entries() {
        let renderers = RenderRegistry::with_builtins();
        let out = renderer_list(&renderers);
        assert!(out.contains("NAME"));
        assert!(out.contains("ACCEPTS"));
        assert!(out.contains("ANIMATES"));
        assert!(out.contains("text_plain"));
    }

    #[test]
    fn options_section_handles_empty() {
        let out = options_section(&[]);
        assert!(out.contains("(none)"));
    }

    #[test]
    fn options_section_lists_schemas() {
        let schema = OptionSchema {
            name: "period",
            type_hint: "\"day\" | \"year\"",
            required: false,
            default: Some("\"day\""),
            description: "Named period.",
        };
        let out = options_section(&[schema]);
        assert!(out.contains("NAME"));
        assert!(out.contains("DEFAULT"));
        assert!(out.contains("period"));
        assert!(out.contains("optional"));
        assert!(out.contains("\"day\""));
        assert!(out.contains("Named period."));
    }

    #[test]
    fn sanitize_replaces_control_chars() {
        let evil = "legit\x1b[2Kspoof";
        assert!(!sanitize(evil).contains('\x1b'));
    }
}
