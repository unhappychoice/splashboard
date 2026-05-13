//! `system_info_locale` — resolved POSIX locale (single Text, no kind).

use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{detect_locale, payload};

pub struct SystemInfoLocale;

impl RealtimeFetcher for SystemInfoLocale {
    fn name(&self) -> &str {
        "system_info_locale"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Resolved POSIX locale (`LC_ALL` > `LC_CTYPE` > `LANG`, fallback `C`)."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("en_US.UTF-8")),
            _ => None,
        }
    }
    fn compute(&self, _ctx: &FetchContext) -> Payload {
        payload(Body::Text(TextData {
            value: detect_locale(),
        }))
    }
}
