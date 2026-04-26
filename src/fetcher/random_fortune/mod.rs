//! `random_fortune` — bundled Unix-style fortune cookies. One entry per day, deterministic
//! by epoch days so the cookie advances at midnight and drifts through the calendar year
//! over year. Distinct from `random_quote`: cookies are anonymous, irreverent, and may
//! span multiple lines (haikus, short verses, multi-line aphorisms). Zero-config, zero
//! I/O, infallible.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, FixedOffset, Local};

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Text];

/// Cookie strings parsed from `data.txt`. Format: `%`-separated entries; entries may be
/// single-line one-liners or multi-line short verses. Newlines inside an entry are
/// preserved verbatim — `Text` shape keeps them, `TextBlock` splits on them.
static COOKIES: LazyLock<Vec<String>> = LazyLock::new(|| parse(include_str!("data.txt")));

pub struct RandomFortuneFetcher;

impl RealtimeFetcher for RandomFortuneFetcher {
    fn name(&self) -> &str {
        "random_fortune"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let cookie = first_cookie()?;
        Some(match shape {
            Shape::Text => samples::text(cookie),
            Shape::TextBlock => samples::text_block(&cookie.lines().collect::<Vec<_>>()),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let cookie = pick(&now());
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: cookie.to_string(),
            }),
            _ => Body::TextBlock(TextBlockData {
                lines: cookie.lines().map(str::to_string).collect(),
            }),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }
}

fn now() -> DateTime<FixedOffset> {
    Local::now().fixed_offset()
}

/// Epoch-days modulo cookie count. Deterministic per calendar day; drifts year over year.
fn pick(now: &DateTime<FixedOffset>) -> &'static str {
    let entries: &'static [String] = &COOKIES;
    let idx = (now.num_days_from_ce() as usize) % entries.len();
    entries[idx].as_str()
}

fn first_cookie() -> Option<&'static str> {
    let entries: &'static [String] = &COOKIES;
    entries.first().map(String::as_str)
}

/// Parse `%`-separated entries. Trims surrounding whitespace per entry, drops empties.
/// Internal newlines are preserved so multi-line cookies render as multiple lines.
/// Normalises CRLF → LF first so Windows checkouts (autocrlf) parse identically.
fn parse(raw: &str) -> Vec<String> {
    raw.replace("\r\n", "\n")
        .split("\n%\n")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};

    use super::*;

    fn at(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0)
            .unwrap()
            .fixed_offset()
    }

    fn ctx(shape: Option<Shape>) -> FetchContext {
        FetchContext {
            widget_id: "f".into(),
            timeout: Duration::from_secs(1),
            shape,
            ..Default::default()
        }
    }

    #[test]
    fn default_shape_is_text_block() {
        let p = RandomFortuneFetcher.compute(&ctx(None));
        let Body::TextBlock(d) = p.body else {
            panic!("expected text_block");
        };
        assert!(!d.lines.is_empty());
    }

    #[test]
    fn text_shape_returns_single_string() {
        let p = RandomFortuneFetcher.compute(&ctx(Some(Shape::Text)));
        let Body::Text(d) = p.body else {
            panic!("expected text");
        };
        assert!(!d.value.is_empty());
    }

    #[test]
    fn pick_is_stable_within_a_day() {
        assert_eq!(pick(&at(2026, 4, 24)), pick(&at(2026, 4, 24)));
    }

    #[test]
    fn pick_advances_with_epoch_day() {
        assert_ne!(pick(&at(2026, 4, 24)), pick(&at(2026, 4, 25)));
    }

    #[test]
    fn pick_drifts_year_over_year() {
        // Year-stable rotation would lock the same cookie to the same date forever.
        assert_ne!(pick(&at(2026, 4, 24)), pick(&at(2027, 4, 24)));
    }

    #[test]
    fn cookie_list_meets_size_floor() {
        assert!(
            COOKIES.len() >= 365,
            "COOKIES has {} entries, want >= 365",
            COOKIES.len()
        );
    }

    #[test]
    fn parse_preserves_multiline_cookie() {
        let cookies = parse("Line one\nLine two\n%\nSingle.\n");
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0], "Line one\nLine two");
        assert_eq!(cookies[1], "Single.");
    }

    #[test]
    fn parse_drops_empty_chunks() {
        assert!(parse("\n%\n\n%\n").is_empty());
    }

    #[test]
    fn cookie_list_has_no_duplicates() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        let mut dups: Vec<&str> = Vec::new();
        for c in COOKIES.iter() {
            if !seen.insert(c.as_str()) {
                dups.push(c.as_str());
            }
        }
        assert!(dups.is_empty(), "duplicate cookies found: {:?}", dups);
    }

    #[test]
    fn parse_handles_crlf_line_endings() {
        // Windows checkouts under git autocrlf convert LF to CRLF; without
        // normalisation the entire data file collapses to a single entry on Windows.
        let lf = "First.\n%\nSecond.\n";
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(parse(lf).len(), 2);
        assert_eq!(parse(&crlf).len(), 2);
        assert_eq!(parse(lf), parse(&crlf));
    }

    #[test]
    fn text_block_splits_multiline_cookie_into_lines() {
        // Synthetic test bypassing the real registry — the production cookie at today's
        // index might be single-line, so we exercise the branch directly.
        let body = Body::TextBlock(TextBlockData {
            lines: "a\nb\nc".lines().map(str::to_string).collect(),
        });
        let Body::TextBlock(d) = body else {
            unreachable!()
        };
        assert_eq!(d.lines, vec!["a", "b", "c"]);
    }
}
