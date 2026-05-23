//! Shared helpers for the `codex_*` family — model pricing, cost math, and the date-tree
//! discovery used by `codex_usage` and `codex_subscription`.
//!
//! Pricing lives in source rather than a vendored file because the OpenAI model surface is
//! small enough to track by hand; bumping a constant is less error-prone than parsing an
//! external table on every fetch. When a new family ships, add a [`PRICE_TABLE`] row matching
//! the model-id prefix.

use std::fs;
use std::path::PathBuf;

/// USD per million tokens for a single OpenAI model family. `cached_input` covers the prompt
/// cache discount (Codex CLI's `cached_input_tokens` lands here, billed at the discounted rate
/// instead of the full `input` rate). Reasoning output is billed at the same rate as regular
/// output today, so it folds into [`Price::output`].
#[derive(Debug, Clone, Copy)]
pub struct Price {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
}

const GPT_5: Price = Price {
    input: 1.25,
    cached_input: 0.125,
    output: 10.0,
};

const GPT_5_MINI: Price = Price {
    input: 0.25,
    cached_input: 0.025,
    output: 2.0,
};

const GPT_5_NANO: Price = Price {
    input: 0.05,
    cached_input: 0.005,
    output: 0.40,
};

const GPT_4_1: Price = Price {
    input: 2.0,
    cached_input: 0.50,
    output: 8.0,
};

const GPT_4_1_MINI: Price = Price {
    input: 0.40,
    cached_input: 0.10,
    output: 1.60,
};

const O3: Price = Price {
    input: 2.0,
    cached_input: 0.50,
    output: 8.0,
};

const O4_MINI: Price = Price {
    input: 1.10,
    cached_input: 0.275,
    output: 4.40,
};

/// Prefix table — longest-prefix entries come first so `gpt-5-mini` doesn't get swallowed by
/// `gpt-5`. Match is on the full lowercased model id.
const PRICE_TABLE: &[(&str, Price)] = &[
    ("gpt-5-nano", GPT_5_NANO),
    ("gpt-5-mini", GPT_5_MINI),
    ("gpt-5", GPT_5),
    ("gpt-4.1-mini", GPT_4_1_MINI),
    ("gpt-4.1", GPT_4_1),
    ("o4-mini", O4_MINI),
    ("o3", O3),
];

pub fn price_for(model: &str) -> Option<Price> {
    let lower = model.to_lowercase();
    PRICE_TABLE
        .iter()
        .find(|(prefix, _)| lower.starts_with(prefix))
        .map(|(_, p)| *p)
}

/// USD cost of a single turn given a model. Unknown models contribute 0 — surfacing them as
/// "free" is friendlier than hiding the underlying token activity behind an error.
pub fn cost_usd(
    model: &str,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
) -> f64 {
    let Some(p) = price_for(model) else {
        return 0.0;
    };
    let per_token = |rate_per_mt: f64, count: u64| (count as f64) * rate_per_mt / 1_000_000.0;
    // OpenAI reports `cached_input_tokens` as a subset of `input_tokens`; bill the cached chunk
    // at the discounted rate and the rest at the full input rate.
    let billable_input = input_tokens.saturating_sub(cached_input_tokens);
    per_token(p.input, billable_input)
        + per_token(p.cached_input, cached_input_tokens)
        + per_token(p.output, output_tokens)
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

/// Codex CLI's per-day session roots. Default `~/.codex/sessions/`; the `CODEX_HOME` env var
/// (Codex's own override) wins so power users with relocated installs stay covered.
pub fn discover_session_dirs() -> Vec<PathBuf> {
    if let Ok(raw) = std::env::var("CODEX_HOME") {
        return vec![PathBuf::from(raw).join("sessions")];
    }
    dirs::home_dir()
        .map(|h| h.join(".codex").join("sessions"))
        .filter(|p| p.is_dir())
        .into_iter()
        .collect()
}

/// Enumerate every `rollout-*.jsonl` under the `YYYY/MM/DD/` tree of each root. Sorted by
/// filename (lexically chronological — Codex's `rollout-<rfc3339>` prefix gives free ordering).
pub fn list_session_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = roots.iter().flat_map(walk_year_month_day).collect();
    files.sort();
    files
}

fn walk_year_month_day(root: &PathBuf) -> Vec<PathBuf> {
    read_subdirs(root)
        .into_iter()
        .flat_map(|year| read_subdirs(&year))
        .flat_map(|month| read_subdirs(&month))
        .flat_map(|day| read_jsonl_files(&day))
        .collect()
}

fn read_subdirs(dir: &PathBuf) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|it| it.flatten())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

fn read_jsonl_files(dir: &PathBuf) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .ok()
        .into_iter()
        .flat_map(|it| it.flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

/// Last path component of a Codex `cwd` field — the bare project name without filesystem noise.
/// Empty cwd falls back to `"(unknown)"` so renderers always get a non-empty key.
pub fn project_name_from_cwd(cwd: &str) -> String {
    PathBuf::from(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(unknown)".into())
}

/// `gpt-5.5` → `gpt-5.5`, `gpt-5-2025-08-12` → `gpt-5`, `gpt-5-mini-2025-08-12` → `gpt-5-mini`.
/// Drops a trailing dash-separated `YYYY-MM-DD` release stamp so the `group_by = model` axis
/// collapses release ids into a family. Anything else passes through unchanged.
pub fn short_model_name(model: &str) -> String {
    let lower = model.to_lowercase();
    strip_trailing_iso_date(&lower).unwrap_or(lower)
}

fn strip_trailing_iso_date(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    // Match `-YYYY-MM-DD` at the end (11 chars total). Cheap manual scan; chrono would pull in
    // strftime parsing for what amounts to a regex literal.
    if bytes.len() < 11 {
        return None;
    }
    let tail = &bytes[bytes.len() - 11..];
    let ok = tail[0] == b'-'
        && tail[5] == b'-'
        && tail[8] == b'-'
        && tail[1..5].iter().all(|b| b.is_ascii_digit())
        && tail[6..8].iter().all(|b| b.is_ascii_digit())
        && tail[9..11].iter().all(|b| b.is_ascii_digit());
    ok.then(|| s[..s.len() - 11].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_for_matches_gpt_5_family_by_prefix() {
        assert!(price_for("gpt-5").is_some());
        assert!(price_for("gpt-5.5").is_some());
        assert!(price_for("gpt-5-mini").is_some());
        assert!(price_for("gpt-5-nano").is_some());
        assert!(price_for("o4-mini").is_some());
    }

    #[test]
    fn price_for_unknown_model_returns_none() {
        assert!(price_for("claude-opus-4-7").is_none());
        assert!(price_for("").is_none());
    }

    #[test]
    fn price_for_longer_prefix_wins_over_shorter() {
        // gpt-5-mini must resolve to GPT_5_MINI, not GPT_5 (it's listed before in PRICE_TABLE).
        let mini = price_for("gpt-5-mini").unwrap();
        assert!(
            (mini.input - 0.25).abs() < 1e-9,
            "expected gpt-5-mini rate, got {}",
            mini.input
        );
    }

    #[test]
    fn cost_usd_zero_for_unknown_model_even_with_tokens() {
        assert_eq!(cost_usd("claude-opus-4-7", 1_000_000, 0, 1_000_000), 0.0);
    }

    #[test]
    fn cost_usd_charges_cached_input_at_discounted_rate() {
        // gpt-5: 1M billable input @ $1.25 + 0M cached + 0M output = $1.25
        let full = cost_usd("gpt-5", 1_000_000, 0, 0);
        assert!((full - 1.25).abs() < 1e-9, "got {full}");
        // Same input but all cached → 1M @ $0.125 = $0.125
        let cached = cost_usd("gpt-5", 1_000_000, 1_000_000, 0);
        assert!((cached - 0.125).abs() < 1e-9, "got {cached}");
    }

    #[test]
    fn cost_usd_splits_partial_cache_against_full_and_cached_rates() {
        // 1M input where 0.5M is cached → 0.5M @ $1.25 + 0.5M @ $0.125 = $0.625 + $0.0625 = $0.6875
        let c = cost_usd("gpt-5", 1_000_000, 500_000, 0);
        assert!((c - 0.6875).abs() < 1e-9, "got {c}");
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
        assert_eq!(project_name_from_cwd(""), "(unknown)");
    }

    #[test]
    fn short_model_name_strips_trailing_iso_date_only() {
        assert_eq!(short_model_name("gpt-5-2025-08-12"), "gpt-5");
        assert_eq!(short_model_name("gpt-5-mini-2025-08-12"), "gpt-5-mini");
        // No trailing date → pass through unchanged.
        assert_eq!(short_model_name("gpt-5.5"), "gpt-5.5");
        assert_eq!(short_model_name("o4-mini"), "o4-mini");
        // A trailing integer that isn't an ISO date must not be stripped.
        assert_eq!(short_model_name("gpt-5-mini-12345"), "gpt-5-mini-12345");
    }

    #[test]
    fn discover_session_dirs_honours_codex_home_env() {
        let _lock = crate::paths::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("CODEX_HOME").ok();
        unsafe { std::env::set_var("CODEX_HOME", "/tmp/codex-test-home") };
        let dirs = discover_session_dirs();
        assert_eq!(dirs.len(), 1);
        assert!(dirs[0].ends_with("codex-test-home/sessions"));
        match prev {
            Some(v) => unsafe { std::env::set_var("CODEX_HOME", v) },
            None => unsafe { std::env::remove_var("CODEX_HOME") },
        }
    }

    #[test]
    fn list_session_files_walks_year_month_day_tree_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let day = tmp.path().join("2026").join("05").join("23");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join("rollout-2026-05-23T01-00-00-b.jsonl"), "").unwrap();
        fs::write(day.join("rollout-2026-05-23T00-00-00-a.jsonl"), "").unwrap();
        // Non-jsonl + extra noise must be ignored.
        fs::write(day.join("note.txt"), "").unwrap();
        let files = list_session_files(&[tmp.path().to_path_buf()]);
        assert_eq!(files.len(), 2);
        // Lexical sort = chronological because filenames are rfc3339-prefixed.
        assert!(
            files[0]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("00-00-00-a")
        );
        assert!(
            files[1]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("01-00-00-b")
        );
    }
}
