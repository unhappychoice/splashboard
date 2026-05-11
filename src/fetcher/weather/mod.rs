//! Weather fetchers. All `Safety::Safe` because every request targets `api.open-meteo.com`
//! — the host is never config-driven. Config supplies coordinates / units / day count, not URLs.
//! No API key is required and no token leaves the machine.

use std::sync::Arc;

pub(crate) mod common;
mod forecast;
mod now;

use super::Fetcher;

pub fn fetchers() -> Vec<Arc<dyn Fetcher>> {
    vec![
        Arc::new(now::WeatherFetcher),
        Arc::new(forecast::WeatherForecastFetcher),
    ]
}
