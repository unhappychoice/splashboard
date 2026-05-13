//! `system_monitor_processes` — top N processes by CPU usage.

use std::sync::Mutex;

use sysinfo::{ProcessesToUpdate, System};

use crate::payload::{Body, EntriesData, Payload, TextBlockData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, payload, top_processes};

pub(crate) const PROCESS_TOP_COUNT: usize = 5;

pub struct SystemMonitorProcesses {
    state: Mutex<System>,
}

impl SystemMonitorProcesses {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        Self {
            state: Mutex::new(sys),
        }
    }
}

impl Default for SystemMonitorProcesses {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemMonitorProcesses {
    fn name(&self) -> &str {
        "system_monitor_processes"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Top five processes by current CPU usage, refreshed every frame. `Entries` pairs each process name with its percentage; `TextBlock` collapses to one process per line."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Entries, Shape::TextBlock]
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => samples::entries(&[
                ("node", "12.4%"),
                ("cargo", "8.1%"),
                ("firefox", "6.3%"),
                ("zsh", "2.1%"),
            ]),
            Shape::TextBlock => samples::text_block(&[
                "node       12.4%",
                "cargo       8.1%",
                "firefox     6.3%",
                "zsh         2.1%",
            ]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let mut sys = self.state.lock().unwrap_or_else(|e| e.into_inner());
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let rows = top_processes(&sys, PROCESS_TOP_COUNT);
        match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::TextBlock => payload(Body::TextBlock(TextBlockData {
                lines: rows.iter().map(|(n, c)| format!("{n}  {c:.1}%")).collect(),
            })),
            _ => payload(Body::Entries(EntriesData {
                items: rows
                    .iter()
                    .map(|(n, c)| entry(n, &format!("{c:.1}%")))
                    .collect(),
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::ctx_with_shape;
    use super::*;

    #[test]
    fn process_top_respects_count_cap() {
        let p = SystemMonitorProcesses::new().compute(&ctx_with_shape(None));
        assert!(matches!(p.body, Body::Entries(_)));
        if let Body::Entries(e) = p.body {
            assert!(e.items.len() <= PROCESS_TOP_COUNT);
        }
    }

    #[test]
    fn process_top_text_block_shape_formats_rows() {
        let p = SystemMonitorProcesses::new().compute(&ctx_with_shape(Some(Shape::TextBlock)));
        assert!(matches!(p.body, Body::TextBlock(_)));
        if let Body::TextBlock(block) = p.body {
            assert!(block.lines.len() <= PROCESS_TOP_COUNT);
            assert!(block.lines.iter().all(|line| line.ends_with('%')));
        }
    }
}
