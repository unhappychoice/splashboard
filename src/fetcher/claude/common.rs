//! Shared helpers for the `claude_*` family — formatting helpers and the JSONL discovery
//! roots used by `claude_code_usage`.
//!
//! Pricing has moved to the shared [`crate::fetcher::llm_pricing`] module so claude and codex
//! consume the same snapshot. See its module doc for the fetch / fallback story.

use std::path::PathBuf;

/// "$12.34" / "$0.05" / "$1.2k" — compact enough to fit a Badge or a Text headline.
pub fn format_cost(usd: f64) -> String {
    if usd >= 1_000.0 {
        format!("${:.1}k", usd / 1_000.0)
    } else if usd >= 10.0 {
        format!("${:.2}", usd)
    } else if usd > 0.0 {
        format!("${:.3}", usd)
    } else {
        "$0.00".into()
    }
}

/// "3.4M" / "812k" / "42" — same compact strategy for raw token counts.
pub fn format_tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{}k", count / 1_000)
    } else {
        count.to_string()
    }
}

/// Claude Code's per-CWD project directories. The CLI uses `~/.claude/projects/<encoded-cwd>/`
/// by default; the older `~/.config/claude/projects/` is still seen on Linux hosts upgraded from
/// pre-3.x. `CLAUDE_CONFIG_DIR` accepts a comma-separated list so power users with multiple
/// installs can point at every root.
pub fn discover_jsonl_dirs() -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("CLAUDE_CONFIG_DIR") {
        return raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(s).join("projects"))
            .collect();
    }
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [home.join(".claude"), home.join(".config").join("claude")]
        .into_iter()
        .map(|p| p.join("projects"))
        .filter(|p| p.is_dir())
        .collect()
}

/// Last path component of a Claude `cwd` field — the bare project name without filesystem noise.
/// Empty cwd falls back to `"(unknown)"` so renderers always get a non-empty key.
pub fn project_name_from_cwd(cwd: &str) -> String {
    PathBuf::from(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(unknown)".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_cost_picks_unit_by_magnitude() {
        assert_eq!(format_cost(0.0), "$0.00");
        assert_eq!(format_cost(0.005), "$0.005");
        assert_eq!(format_cost(2.50), "$2.500");
        assert_eq!(format_cost(12.34), "$12.34");
        assert_eq!(format_cost(1234.0), "$1.2k");
    }

    #[test]
    fn format_tokens_uses_m_and_k_suffixes() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(42), "42");
        assert_eq!(format_tokens(1_500), "1k");
        assert_eq!(format_tokens(3_400_000), "3.4M");
    }

    #[test]
    fn project_name_from_cwd_takes_last_component() {
        assert_eq!(
            project_name_from_cwd("/home/owner/.ghq/github.com/unhappychoice/splashboard"),
            "splashboard"
        );
    }

    #[test]
    fn project_name_from_cwd_handles_empty_path() {
        assert_eq!(project_name_from_cwd(""), "(unknown)");
    }

    #[test]
    fn discover_jsonl_dirs_honours_env_override() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("CLAUDE_CONFIG_DIR").ok();
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/a, /tmp/b") };
        let dirs = discover_jsonl_dirs();
        assert_eq!(dirs.len(), 2);
        assert!(dirs[0].ends_with("a/projects"));
        assert!(dirs[1].ends_with("b/projects"));
        match prev {
            Some(v) => unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", v) },
            None => unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") },
        }
    }
}
