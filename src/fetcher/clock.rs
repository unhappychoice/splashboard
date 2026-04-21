use std::fmt::Write;

use async_trait::async_trait;
use chrono::Local;

use crate::payload::{BignumData, Body, Payload};

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
            body: Body::Bignum(BignumData { text }),
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

    #[tokio::test]
    async fn default_format_is_hh_mm() {
        let p = ClockFetcher.fetch(&ctx(None)).await.unwrap();
        match p.body {
            Body::Bignum(d) => {
                assert_eq!(d.text.len(), 5, "{:?} should be HH:MM", d.text);
                assert_eq!(d.text.chars().nth(2), Some(':'));
            }
            _ => panic!("expected bignum body"),
        }
    }

    #[tokio::test]
    async fn custom_format_is_honored() {
        let p = ClockFetcher.fetch(&ctx(Some("%Y"))).await.unwrap();
        match p.body {
            Body::Bignum(d) => assert_eq!(d.text.len(), 4, "{:?} should be YYYY", d.text),
            _ => panic!("expected bignum body"),
        }
    }

    #[tokio::test]
    async fn invalid_format_falls_back_without_panicking() {
        // `%Q` is not a recognised strftime directive; chrono's formatter returns `fmt::Error`
        // for it, which would panic through `to_string()`. Our wrapper must return a valid
        // default-formatted string instead.
        let p = ClockFetcher.fetch(&ctx(Some("%Q"))).await.unwrap();
        match p.body {
            Body::Bignum(d) => {
                assert_eq!(d.text.len(), 5, "expected fallback HH:MM, got {:?}", d.text);
                assert_eq!(d.text.chars().nth(2), Some(':'));
            }
            _ => panic!("expected bignum body"),
        }
    }

    #[tokio::test]
    async fn literal_percent_in_format_does_not_crash() {
        // Plain literals are fine.
        let p = ClockFetcher.fetch(&ctx(Some("now: %H:%M"))).await.unwrap();
        match p.body {
            Body::Bignum(d) => assert!(d.text.starts_with("now: ")),
            _ => panic!("expected bignum body"),
        }
    }
}
