//! Shared helpers for the `claude_*` family — model pricing, cost math, and the JSONL discovery
//! roots used by `claude_code_usage`.
//!
//! Pricing lives in source rather than a vendored JSON file because the set of Claude models is
//! small and changes rarely; bumping a constant is less error-prone than parsing an external table
//! on every fetch. When a new family ships, add a [`PRICE_TABLE`] row matching the model-id prefix.

use std::path::PathBuf;

/// USD per million tokens for a single Claude model family. Cache write tiers map to Anthropic's
/// `ephemeral_5m` / `ephemeral_1h` breakouts in the usage payload; `cache_read` covers the
/// flat-rate cache-read line.
#[derive(Debug, Clone, Copy)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

const OPUS: Price = Price {
    input: 15.0,
    output: 75.0,
    cache_write_5m: 18.75,
    cache_write_1h: 30.0,
    cache_read: 1.50,
};

const SONNET: Price = Price {
    input: 3.0,
    output: 15.0,
    cache_write_5m: 3.75,
    cache_write_1h: 6.0,
    cache_read: 0.30,
};

const HAIKU: Price = Price {
    input: 1.0,
    output: 5.0,
    cache_write_5m: 1.25,
    cache_write_1h: 2.0,
    cache_read: 0.10,
};

/// Prefix table — first match wins, longest-prefix entries appear first so `opus` doesn't shadow
/// a hypothetical `opus-mini`. Match is on the full lowercased model id.
const PRICE_TABLE: &[(&str, Price)] = &[
    ("claude-opus", OPUS),
    ("claude-sonnet", SONNET),
    ("claude-haiku", HAIKU),
    // Legacy date-stamped names still seen in older JSONL sessions.
    ("claude-3-5-sonnet", SONNET),
    ("claude-3-5-haiku", HAIKU),
    ("claude-3-opus", OPUS),
    ("claude-3-sonnet", SONNET),
    ("claude-3-haiku", HAIKU),
];

pub fn price_for(model: &str) -> Option<Price> {
    let lower = model.to_lowercase();
    PRICE_TABLE
        .iter()
        .find(|(prefix, _)| lower.starts_with(prefix))
        .map(|(_, p)| *p)
}

/// USD cost of a usage row given a model. Unknown models contribute 0 — surfacing them as
/// "free" is friendlier than hiding the underlying token activity behind an error.
#[allow(clippy::too_many_arguments)]
pub fn cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
) -> f64 {
    let Some(p) = price_for(model) else {
        return 0.0;
    };
    let per_token = |rate_per_mt: f64, count: u64| (count as f64) * rate_per_mt / 1_000_000.0;
    per_token(p.input, input_tokens)
        + per_token(p.output, output_tokens)
        + per_token(p.cache_write_5m, cache_write_5m)
        + per_token(p.cache_write_1h, cache_write_1h)
        + per_token(p.cache_read, cache_read)
}

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
    fn price_for_matches_opus_sonnet_haiku_by_prefix() {
        assert!(price_for("claude-opus-4-7").is_some());
        assert!(price_for("claude-sonnet-4-6").is_some());
        assert!(price_for("claude-haiku-4-5-20251001").is_some());
        assert!(price_for("claude-3-5-sonnet-20241022").is_some());
    }

    #[test]
    fn price_for_unknown_model_returns_none() {
        assert!(price_for("gpt-4-turbo").is_none());
        assert!(price_for("").is_none());
    }

    #[test]
    fn price_for_is_case_insensitive() {
        assert!(price_for("CLAUDE-OPUS-4-7").is_some());
    }

    #[test]
    fn cost_usd_zero_for_unknown_model_even_with_tokens() {
        assert_eq!(cost_usd("gpt-4", 1_000_000, 1_000_000, 0, 0, 0), 0.0);
    }

    #[test]
    fn cost_usd_sums_per_tier_rates_against_million_tokens() {
        // Opus: 1M input @ $15 + 1M output @ $75 = $90 exactly.
        let c = cost_usd("claude-opus-4-7", 1_000_000, 1_000_000, 0, 0, 0);
        assert!((c - 90.0).abs() < 1e-6, "expected ~$90, got {c}");
    }

    #[test]
    fn cost_usd_charges_cache_tiers() {
        // Sonnet cache_write_1h: 1M @ $6 = $6.
        let c = cost_usd("claude-sonnet-4-6", 0, 0, 0, 1_000_000, 0);
        assert!((c - 6.0).abs() < 1e-6);
    }

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
