//! `deariary_*` fetcher family — surfaces deariary.com's auto-generated diary entries.
//!
//! deariary.com aggregates external tools (GitHub, Calendar, Slack, Linear, Todoist, ...) into
//! a written entry each morning. Every fetcher in this family targets the fixed host
//! `api.deariary.com`, so the bearer token (`options.token` or `DEARIARY_TOKEN` env) can
//! never be redirected to an attacker-controlled origin — Safety::Safe.
//!
//! Streak / "did I write today" semantics deliberately don't apply: deariary positions itself
//! as auto-generated so there's nothing for the user to miss.

mod client;
pub mod on_this_day;
pub mod recent;
pub mod today;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(today::DeariaryToday),
        Arc::new(recent::DeariaryRecent),
        Arc::new(on_this_day::DeariaryOnThisDay),
    ]
}
