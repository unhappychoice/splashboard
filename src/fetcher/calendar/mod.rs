//! `calendar_*` fetcher family — calendar-shaped data (today's events, upcoming dates,
//! month grids).
//!
//! Currently houses `calendar_holidays`; designed to absorb the planned `calendar_ical`,
//! `calendar_google`, and `calendar_today_events` siblings (catalog #68) under one family.

pub mod holidays;

use std::sync::Arc;

use crate::fetcher::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![Arc::new(holidays::CalendarHolidaysFetcher)]
}
