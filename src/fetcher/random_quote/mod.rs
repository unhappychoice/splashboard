//! `random_quote` — a bundled daily quote. Selection is deterministic per calendar day
//! (epoch days modulo the built-in quote count), so every `splashboard` invocation on
//! the same day sees the same line and the quote advances automatically at midnight.
//! Indexing by epoch days (not day-of-year) means the cycle drifts year over year, so
//! the same date doesn't always return the same quote. Zero-config, zero I/O, infallible.

use std::sync::LazyLock;

use chrono::{DateTime, Datelike, FixedOffset, Local};

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::payload::{Body, Payload, TextBlockData, TextData};
use crate::render::Shape;
use crate::samples;

const SHAPES: &[Shape] = &[Shape::TextBlock, Shape::Text];

/// Quote / author pairs parsed from `data.txt`. Format: `%`-separated entries; each entry
/// is the quote text optionally followed by a final line starting with `— ` for the author.
/// Entries with no `—` line are treated as anonymous (author = "Anonymous").
static QUOTES: LazyLock<Vec<(String, String)>> = LazyLock::new(|| parse(include_str!("data.txt")));

pub struct RandomQuoteFetcher;

impl RealtimeFetcher for RandomQuoteFetcher {
    fn name(&self) -> &str {
        "random_quote"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        let (q, a) = first_quote()?;
        Some(match shape {
            Shape::Text => samples::text(&single_line(q, a)),
            Shape::TextBlock => samples::text_block(&[q, &format!("— {a}")]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let now = now();
        let (quote, author) = pick(&now);
        let body = match ctx.shape.unwrap_or(Shape::TextBlock) {
            Shape::Text => Body::Text(TextData {
                value: single_line(quote, author),
            }),
            _ => Body::TextBlock(TextBlockData {
                lines: vec![quote.to_string(), format!("— {author}")],
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

/// Epoch-days modulo quote count — deterministic, rolls over at midnight, drifts through
/// the calendar year over year (unlike day-of-year, which would lock the same quote to
/// the same date every year).
fn pick(now: &DateTime<FixedOffset>) -> (&'static str, &'static str) {
    let entries: &'static [(String, String)] = &QUOTES;
    let idx = (now.num_days_from_ce() as usize) % entries.len();
    let (q, a) = &entries[idx];
    (q.as_str(), a.as_str())
}

fn first_quote() -> Option<(&'static str, &'static str)> {
    let entries: &'static [(String, String)] = &QUOTES;
    entries.first().map(|(q, a)| (q.as_str(), a.as_str()))
}

fn single_line(quote: &str, author: &str) -> String {
    format!("{quote} — {author}")
}

/// Parse `%`-separated entries. Each entry's last line starting with `— ` is the author;
/// everything before it (joined by spaces, trimmed) is the quote. Empty entries dropped.
/// Normalises CRLF → LF first so Windows checkouts (autocrlf) parse identically.
fn parse(raw: &str) -> Vec<(String, String)> {
    raw.replace("\r\n", "\n")
        .split("\n%\n")
        .filter_map(parse_entry)
        .collect()
}

fn parse_entry(chunk: &str) -> Option<(String, String)> {
    let trimmed = chunk.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lines: Vec<&str> = trimmed.lines().collect();
    let (author, quote_lines) = match lines.split_last() {
        Some((last, rest)) if last.trim_start().starts_with("— ") => (
            last.trim_start()
                .trim_start_matches("— ")
                .trim()
                .to_string(),
            rest,
        ),
        _ => ("Anonymous".to_string(), lines.as_slice()),
    };
    let quote = quote_lines
        .iter()
        .map(|s| s.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if quote.is_empty() {
        None
    } else {
        Some((quote, author))
    }
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
            widget_id: "q".into(),
            timeout: Duration::from_secs(1),
            shape,
            ..Default::default()
        }
    }

    #[test]
    fn default_shape_is_text_block() {
        let p = RandomQuoteFetcher.compute(&ctx(None));
        let Body::TextBlock(d) = p.body else {
            panic!("expected text_block");
        };
        assert_eq!(d.lines.len(), 2);
        assert!(d.lines[1].starts_with("— "));
    }

    #[test]
    fn text_shape_joins_with_dash() {
        let p = RandomQuoteFetcher.compute(&ctx(Some(Shape::Text)));
        let Body::Text(d) = p.body else {
            panic!("expected text");
        };
        assert!(d.value.contains(" — "));
    }

    #[test]
    fn pick_is_stable_within_a_day() {
        let a = pick(&at(2026, 4, 24));
        let b = pick(&at(2026, 4, 24));
        assert_eq!(a, b);
    }

    #[test]
    fn pick_advances_with_epoch_day() {
        // Consecutive days index different slots (unless we land on a modulo boundary,
        // which would only happen once per cycle of length QUOTES.len()).
        let a = pick(&at(2026, 4, 24));
        let b = pick(&at(2026, 4, 25));
        assert_ne!(a, b);
    }

    #[test]
    fn pick_drifts_year_over_year() {
        // Same calendar date one year apart should NOT return the same quote, otherwise
        // we'd be locked into a 366-day cycle aligned to the calendar.
        let a = pick(&at(2026, 4, 24));
        let b = pick(&at(2027, 4, 24));
        assert_ne!(
            a, b,
            "year-over-year drift broken — quote count may divide 365"
        );
    }

    #[test]
    fn quote_list_meets_size_floor() {
        // 365+ guarantees no within-year repeat.
        assert!(
            QUOTES.len() >= 365,
            "QUOTES has {} entries, want >= 365",
            QUOTES.len()
        );
    }

    #[test]
    fn parse_entry_extracts_author() {
        let (q, a) = parse_entry("Hello world.\n— Someone").unwrap();
        assert_eq!(q, "Hello world.");
        assert_eq!(a, "Someone");
    }

    #[test]
    fn parse_entry_falls_back_to_anonymous() {
        let (q, a) = parse_entry("Just a line.").unwrap();
        assert_eq!(q, "Just a line.");
        assert_eq!(a, "Anonymous");
    }

    #[test]
    fn parse_entry_joins_multiline_quote() {
        let (q, a) = parse_entry("Line one\nLine two\n— Poet").unwrap();
        assert_eq!(q, "Line one Line two");
        assert_eq!(a, "Poet");
    }

    #[test]
    fn parse_drops_empty_chunks() {
        assert!(parse("\n%\n\n%\n").is_empty());
    }

    #[test]
    fn quote_list_has_no_duplicates() {
        // Catch exact-text duplicates regardless of author so reordering or
        // adding-without-checking doesn't quietly inflate the count past
        // entries that already exist.
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        let mut dups: Vec<&str> = Vec::new();
        for (q, _) in QUOTES.iter() {
            if !seen.insert(q.as_str()) {
                dups.push(q.as_str());
            }
        }
        assert!(dups.is_empty(), "duplicate quotes found: {:?}", dups);
    }

    #[test]
    fn parse_handles_crlf_line_endings() {
        // Windows checkouts under git autocrlf convert LF to CRLF; the parser must
        // still recover the same entry count, otherwise size-floor and rotation
        // tests fail on Windows runners.
        let lf = "First.\n— A\n%\nSecond.\n— B\n";
        let crlf = lf.replace('\n', "\r\n");
        assert_eq!(parse(lf).len(), 2);
        assert_eq!(parse(&crlf).len(), 2);
        assert_eq!(parse(lf), parse(&crlf));
    }
}
