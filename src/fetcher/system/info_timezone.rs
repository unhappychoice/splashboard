//! `system_info_timezone` — IANA timezone (single Text, no kind).

use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{detect_timezone, payload};

pub struct SystemInfoTimezone;

impl RealtimeFetcher for SystemInfoTimezone {
    fn name(&self) -> &str {
        "system_info_timezone"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "IANA timezone name, resolved from `$TZ` or the `/etc/localtime` symlink target (e.g. `Asia/Tokyo`)."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("Asia/Tokyo")),
            _ => None,
        }
    }
    fn compute(&self, _ctx: &FetchContext) -> Payload {
        payload(Body::Text(TextData {
            value: detect_timezone(),
        }))
    }
}
