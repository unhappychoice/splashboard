//! `system_monitor_load` — 1 / 5 / 15-minute load averages.

use sysinfo::System;

use crate::payload::{Body, EntriesData, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, format_load, load_line, payload};

pub struct SystemMonitorLoad;

impl RealtimeFetcher for SystemMonitorLoad {
    fn name(&self) -> &str {
        "system_monitor_load"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Unix 1 / 5 / 15-minute load averages. `Text` joins the three values on one line; `Entries` splits them into separate rows. Reads as `\"n/a (windows)\"` on Windows, which has no equivalent counter."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text, Shape::Entries]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("0.42  0.38  0.31"),
            Shape::Entries => {
                samples::entries(&[("1min", "0.42"), ("5min", "0.38"), ("15min", "0.31")])
            }
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let la = System::load_average();
        match ctx.shape.unwrap_or(Shape::Text) {
            Shape::Entries => payload(Body::Entries(EntriesData {
                items: vec![
                    entry("1min", &format_load(la.one)),
                    entry("5min", &format_load(la.five)),
                    entry("15min", &format_load(la.fifteen)),
                ],
            })),
            _ => payload(Body::Text(TextData {
                value: load_line(la.one, la.five, la.fifteen),
            })),
        }
    }
}
