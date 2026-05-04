//! Emit a `_presets.json` index describing every bundled template, sourced directly from
//! `splashboard::templates::TEMPLATES`. Drives the dynamic gallery on `presets.mdx` so the
//! page descriptions can never drift from the install-picker descriptions.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use splashboard::templates::{TEMPLATES, TemplateContext};

#[derive(Debug, Serialize)]
struct PresetEntry {
    slug: String,
    description: String,
    context: PresetContext,
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum PresetContext {
    Home,
    Project,
}

impl From<TemplateContext> for PresetContext {
    fn from(ctx: TemplateContext) -> Self {
        match ctx {
            TemplateContext::Home => PresetContext::Home,
            TemplateContext::Project => PresetContext::Project,
        }
    }
}

pub fn run(out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    let entries: Vec<PresetEntry> = TEMPLATES
        .iter()
        .map(|t| PresetEntry {
            slug: t.name.to_string(),
            description: t.description.to_string(),
            context: t.context.into(),
        })
        .collect();
    let index_path = out_dir.join("_presets.json");
    let index_json = serde_json::to_string_pretty(&entries).context("serialize presets index")?;
    fs::write(&index_path, index_json)
        .with_context(|| format!("write {}", index_path.display()))?;
    println!("wrote {}", index_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_writes_an_entry_for_every_template() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "splashboard-preset-index-{unique}-{}",
            std::process::id()
        ));

        run(&dir).unwrap();
        let body = fs::read_to_string(dir.join("_presets.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), TEMPLATES.len());
        for (entry, template) in arr.iter().zip(TEMPLATES.iter()) {
            assert_eq!(entry["slug"], template.name);
            assert_eq!(entry["description"], template.description);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
