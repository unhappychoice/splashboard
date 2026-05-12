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
        let mut sys = self.state.lock().expect("process state mutex poisoned");
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
