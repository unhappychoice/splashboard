//! Shared LLM pricing helper for the `codex_*` and `claude_*` families.
//!
//! Fetches `data/llm_pricing.json` from this repository's `main` branch
//! (`raw.githubusercontent.com/unhappychoice/splashboard/main/data/llm_pricing.json`) on every
//! call. The host is hardcoded — config never controls where the request goes, which keeps the
//! callers (codex_usage, claude_code_usage) in the `Safety::Safe` class.
//!
//! Why HTTP at all when we ship the file in our own repo:
//! - splashboard binaries persist for months between releases; LiteLLM ships new model
//!   identifiers weekly. HTTP fetch lets old binaries see new prices without a rebuild.
//! - The fetcher's own disk cache (5-minute TTL on both consumers today) absorbs the redundancy
//!   — the price lookup only fires when the outer cache is stale, not on every render frame.
//!
//! Why we sync into this repo instead of fetching LiteLLM directly:
//! - LiteLLM's schema changes are caught by the GitHub Actions sync PR before they reach users.
//! - LiteLLM outages don't take down our cost rollups — the snapshot in our repo keeps serving.
//!
//! On any failure (network down, parse error, schema drift) the helper falls back to a tiny
//! `EMBEDDED_FLOOR` covering the current model families. Unknown models still contribute $0
//! cost, mirroring the original hardcoded behaviour.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

const SOURCE_URL: &str =
    "https://raw.githubusercontent.com/unhappychoice/splashboard/main/data/llm_pricing.json";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// USD per million tokens for a single LLM model.
///
/// `cache_write_5m` / `cache_write_1h` only apply to Anthropic's ephemeral cache tiers; for
/// OpenAI models they stay at 0 and the caller's cost math drops the term naturally.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Default)]
pub struct Price {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write_5m: f64,
    #[serde(default)]
    pub cache_write_1h: f64,
}

/// Snapshot of pricing for every model the splashboard repo currently tracks.
pub type PriceMap = HashMap<String, Price>;

/// Fetch the latest snapshot. Returns the embedded floor on any failure so callers don't have
/// to thread `Result` through every cost-math call site.
pub async fn fetch_pricing(http: &Client) -> PriceMap {
    fetch_remote(http)
        .await
        .unwrap_or_else(|_| embedded_floor())
}

async fn fetch_remote(http: &Client) -> Result<PriceMap, String> {
    let res = http
        .get(SOURCE_URL)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status()));
    }
    let body: Snapshot = res.json().await.map_err(|e| format!("parse: {e}"))?;
    Ok(body.models)
}

#[derive(Deserialize)]
struct Snapshot {
    #[serde(default)]
    models: PriceMap,
}

/// Longest-prefix lookup over the snapshot. `gpt-5-2025-08-12` resolves to `gpt-5` when the
/// dated variant isn't a key, so newly-released identifiers within a known family stay priced
/// until the next Actions sync ships an exact match.
pub fn price_for<'a>(prices: &'a PriceMap, model: &str) -> Option<&'a Price> {
    let lower = model.to_lowercase();
    if let Some(p) = prices.get(&lower) {
        return Some(p);
    }
    // Prefix match against lowercased keys too — `data/llm_pricing.json` keys happen to be
    // lowercase today but the lookup shouldn't quietly miss if a future LiteLLM sync ships a
    // mixed-case identifier.
    prices
        .iter()
        .filter(|(k, _)| !k.is_empty() && lower.starts_with(&k.to_lowercase()))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, p)| p)
}

/// USD cost of a single token-usage row. Unknown models contribute 0 — surfacing them as
/// "free" is friendlier than hiding the underlying token activity behind an error.
pub fn cost_usd(
    prices: &PriceMap,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
) -> f64 {
    let Some(p) = price_for(prices, model) else {
        return 0.0;
    };
    let per_token = |rate: f64, count: u64| (count as f64) * rate / 1_000_000.0;
    // `cache_read` covers the discounted cached-input tier; the input rate applies to the
    // non-cached portion only. OpenAI reports `cached_input_tokens` as a subset of
    // `input_tokens`, so the caller subtracts it before passing into `input_tokens` here.
    per_token(p.input, input_tokens)
        + per_token(p.output, output_tokens)
        + per_token(p.cache_read, cache_read)
        + per_token(p.cache_write_5m, cache_write_5m)
        + per_token(p.cache_write_1h, cache_write_1h)
}

/// Bare minimum for cold-start (no network, no cache) so cost columns aren't entirely zero.
/// The snapshot in `data/llm_pricing.json` is the authoritative source; this only covers the
/// shipping headline models. Anything missing here will still get a `Some(Price)` via the
/// next successful HTTP fetch.
pub fn embedded_floor() -> PriceMap {
    static FLOOR: OnceLock<PriceMap> = OnceLock::new();
    FLOOR
        .get_or_init(|| {
            let mut m = PriceMap::new();
            m.insert(
                "gpt-5".into(),
                Price {
                    input: 1.25,
                    output: 10.0,
                    cache_read: 0.125,
                    ..Default::default()
                },
            );
            m.insert(
                "gpt-5-mini".into(),
                Price {
                    input: 0.25,
                    output: 2.0,
                    cache_read: 0.025,
                    ..Default::default()
                },
            );
            m.insert(
                "claude-opus-4-7".into(),
                Price {
                    input: 5.0,
                    output: 25.0,
                    cache_read: 0.5,
                    cache_write_5m: 6.25,
                    cache_write_1h: 10.0,
                },
            );
            m.insert(
                "claude-sonnet-4-6".into(),
                Price {
                    input: 3.0,
                    output: 15.0,
                    cache_read: 0.3,
                    cache_write_5m: 3.75,
                    cache_write_1h: 6.0,
                },
            );
            m.insert(
                "claude-haiku-4-5".into(),
                Price {
                    input: 1.0,
                    output: 5.0,
                    cache_read: 0.1,
                    cache_write_5m: 1.25,
                    cache_write_1h: 2.0,
                },
            );
            m
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_prices() -> PriceMap {
        let mut m = PriceMap::new();
        m.insert(
            "gpt-5".into(),
            Price {
                input: 1.25,
                output: 10.0,
                cache_read: 0.125,
                ..Default::default()
            },
        );
        m.insert(
            "gpt-5-mini".into(),
            Price {
                input: 0.25,
                output: 2.0,
                cache_read: 0.025,
                ..Default::default()
            },
        );
        m.insert(
            "claude-opus-4-7".into(),
            Price {
                input: 5.0,
                output: 25.0,
                cache_read: 0.5,
                cache_write_5m: 6.25,
                cache_write_1h: 10.0,
            },
        );
        m
    }

    #[test]
    fn price_for_exact_match_wins() {
        let p = sample_prices();
        assert_eq!(price_for(&p, "gpt-5").map(|x| x.input), Some(1.25));
    }

    #[test]
    fn price_for_falls_back_to_longest_prefix() {
        // `gpt-5-2025-08-12` isn't an exact key; longest prefix is `gpt-5`.
        let p = sample_prices();
        let row = price_for(&p, "gpt-5-2025-08-12").expect("must resolve via prefix");
        assert_eq!(row.input, 1.25);
    }

    #[test]
    fn price_for_longest_prefix_wins_over_shorter() {
        // `gpt-5-mini-2025-08-12` must resolve to `gpt-5-mini`, not `gpt-5`.
        let p = sample_prices();
        let row = price_for(&p, "gpt-5-mini-2025-08-12").expect("must resolve via prefix");
        assert_eq!(row.input, 0.25);
    }

    #[test]
    fn price_for_is_case_insensitive() {
        let p = sample_prices();
        assert!(price_for(&p, "GPT-5").is_some());
        assert!(price_for(&p, "Claude-Opus-4-7").is_some());
    }

    #[test]
    fn price_for_unknown_model_returns_none() {
        let p = sample_prices();
        assert!(price_for(&p, "mistral-large").is_none());
        assert!(price_for(&p, "").is_none());
    }

    #[test]
    fn cost_usd_zero_for_unknown_model_even_with_tokens() {
        let p = sample_prices();
        assert_eq!(
            cost_usd(&p, "mistral-large", 1_000_000, 1_000_000, 0, 0, 0),
            0.0
        );
    }

    #[test]
    fn cost_usd_sums_per_tier_rates() {
        // gpt-5: 1M input @ $1.25 + 1M output @ $10.00 = $11.25
        let p = sample_prices();
        let c = cost_usd(&p, "gpt-5", 1_000_000, 1_000_000, 0, 0, 0);
        assert!((c - 11.25).abs() < 1e-9, "expected $11.25, got {c}");
    }

    #[test]
    fn cost_usd_charges_anthropic_cache_write_tiers() {
        // claude-opus-4-7 cache_write_1h: 1M @ $10 = $10
        let p = sample_prices();
        let c = cost_usd(&p, "claude-opus-4-7", 0, 0, 0, 0, 1_000_000);
        assert!((c - 10.0).abs() < 1e-9, "expected $10, got {c}");
    }

    #[test]
    fn embedded_floor_covers_current_headline_models() {
        let f = embedded_floor();
        assert!(f.contains_key("gpt-5"));
        assert!(f.contains_key("gpt-5-mini"));
        assert!(f.contains_key("claude-opus-4-7"));
        assert!(f.contains_key("claude-sonnet-4-6"));
        assert!(f.contains_key("claude-haiku-4-5"));
    }

    #[test]
    fn snapshot_parses_repo_seed() {
        // Guarantees our own `data/llm_pricing.json` stays loadable. If schema drift in the
        // sync workflow ever produces an incompatible file, this test catches it before the
        // PR can merge.
        let raw = include_str!("../../data/llm_pricing.json");
        let snap: Snapshot = serde_json::from_str(raw).expect("seed must parse");
        assert!(snap.models.contains_key("gpt-5"));
        assert!(snap.models.contains_key("claude-opus-4-7"));
    }
}
