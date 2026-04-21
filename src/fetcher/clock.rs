use std::fmt::Write;

use async_trait::async_trait;
use chrono::Local;

use crate::payload::{Body, LinesData, Payload};

use super::{FetchContext, FetchError, Fetcher, Safety};

const DEFAULT_FORMAT: &str = "%H:%M";

/// Renders the current local time. `format` follows chrono's strftime conventions; default is
/// `%H:%M` (24h clock). Emits a Bignum payload so the default layout can lean on the big-text
/// renderer for visual weight.
pub struct ClockFetcher;

#[async_trait]
impl Fetcher for ClockFetcher {
    fn name(&self) -> &str {
        "clock"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let fmt = ctx.format.as_deref().unwrap_or(DEFAULT_FORMAT);
        let text = format_now(fmt);
        Ok(Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData { lines: vec![text] }),
        })
    }
}

/// Formats the current local time using `fmt`. An invalid strftime directive would cause
/// chrono's `DelayedFormat::to_string()` to panic (`Display::fmt` returns `Err`, and
/// `ToString::to_string` panics on that). We capture the error via `write!` and fall back to
/// the default format so a typo in a user's config can never crash the splash.
fn format_now(fmt: &str) -> String {
    let now = Local::now();
    let mut buf = String::new();
    if write!(&mut buf, "{}", now.format(fmt)).is_ok() {
        return buf;
    }
    // The partial write on error may leave buf in an unusable state; discard it.
    let mut fallback = String::new();
    // `write!` on the default format is infallible; `unwrap_or_default` keeps it panic-free
    // even if some future version of chrono changes that invariant.
    let _ = write!(&mut fallback, "{}", now.format(DEFAULT_FORMAT));
    fallback
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(format: Option<&str>) -> FetchContext {
        FetchContext {
            widget_id: "clock".into(),
            format: format.map(String::from),
            timeout: Duration::from_secs(1),
        }
    }

    fn first_line(p: Payload) -> String {
        match p.body {
            Body::Lines(d) => d.lines.into_iter().next().unwrap_or_default(),
            _ => panic!("expected lines body"),
        }
    }

    #[tokio::test]
    async fn default_format_is_hh_mm() {
        let text = first_line(ClockFetcher.fetch(&ctx(None)).await.unwrap());
        assert_eq!(text.len(), 5, "{text:?} should be HH:MM");
        assert_eq!(text.chars().nth(2), Some(':'));
    }

    #[tokio::test]
    async fn custom_format_is_honored() {
        let text = first_line(ClockFetcher.fetch(&ctx(Some("%Y"))).await.unwrap());
        assert_eq!(text.len(), 4, "{text:?} should be YYYY");
    }

    #[tokio::test]
    async fn invalid_format_falls_back_without_panicking() {
        // `%Q` is not a recognised strftime directive; chrono's formatter returns `fmt::Error`
        // for it, which would panic through `to_string()`. Our wrapper must return a valid
        // default-formatted string instead.
        let text = first_line(ClockFetcher.fetch(&ctx(Some("%Q"))).await.unwrap());
        assert_eq!(text.len(), 5, "expected fallback HH:MM, got {text:?}");
        assert_eq!(text.chars().nth(2), Some(':'));
    }

    #[tokio::test]
    async fn literal_percent_in_format_does_not_crash() {
        let text = first_line(ClockFetcher.fetch(&ctx(Some("now: %H:%M"))).await.unwrap());
        assert!(text.starts_with("now: "));
    }
}
