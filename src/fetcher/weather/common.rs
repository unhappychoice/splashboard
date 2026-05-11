//! Shared helpers for the `weather_*` family. The Open-Meteo API host is hardcoded so any
//! fetcher routed through here keeps the `Safety::Safe` classification — config supplies
//! coordinates, not URLs.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use crate::payload::{BadgeData, Body, Payload, Status};

pub const API_BASE: &str = "https://api.open-meteo.com/v1/forecast";
pub const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

impl Units {
    pub fn temperature_label(self) -> &'static str {
        match self {
            Self::Metric => "°C",
            Self::Imperial => "°F",
        }
    }

    pub fn wind_label(self) -> &'static str {
        match self {
            Self::Metric => "m/s",
            Self::Imperial => "mph",
        }
    }

    /// Precipitation unit suffix. Open-Meteo reports mm on metric, inches on imperial when
    /// `precipitation_unit=inch` is passed; we keep the metric default and label accordingly.
    pub fn precipitation_label(self) -> &'static str {
        match self {
            Self::Metric => "mm",
            Self::Imperial => "in",
        }
    }
}

/// WMO weather interpretation codes (Open-Meteo uses the standard table).
pub fn weather_description(code: u16) -> (&'static str, &'static str) {
    match code {
        0 => ("🌞", "clear"),
        1 => ("🌤", "mostly clear"),
        2 => ("⛅", "partly cloudy"),
        3 => ("☁", "overcast"),
        45 | 48 => ("🌫", "fog"),
        51 | 53 | 55 => ("🌦", "drizzle"),
        56 | 57 => ("🌧", "freezing drizzle"),
        61 | 63 | 65 => ("🌧", "rain"),
        66 | 67 => ("🌧", "freezing rain"),
        71 | 73 | 75 | 77 => ("🌨", "snow"),
        80..=82 => ("🌦", "rain showers"),
        85 | 86 => ("🌨", "snow showers"),
        95 => ("⛈", "thunderstorm"),
        96 | 99 => ("⛈", "thunderstorm w/ hail"),
        _ => ("🌡", "unknown"),
    }
}

/// Severity badge for a single WMO code. Multi-day fetchers compute it over the worst code
/// in their window so the headline reflects the riskiest day.
pub fn weather_badge(code: u16) -> BadgeData {
    let (status, label) = match code {
        95 | 96 | 99 => (Status::Error, "thunderstorm"),
        56 | 57 | 66 | 67 => (Status::Warn, "freezing"),
        65 | 75 | 82 | 86 => (Status::Warn, "heavy precip"),
        61 | 63 | 71 | 73 | 80 | 81 | 85 => (Status::Warn, "precip"),
        _ => {
            let (_, desc) = weather_description(code);
            (Status::Ok, desc)
        }
    };
    BadgeData {
        status,
        label: label.into(),
    }
}

/// Severity rank — higher = worse. Used to pick the worst weather code from a forecast window
/// for the `Badge` shape. Matches the buckets in `weather_badge`.
pub fn severity_rank(code: u16) -> u8 {
    match code {
        95 | 96 | 99 => 3,
        56 | 57 | 66 | 67 | 65 | 75 | 82 | 86 => 2,
        61 | 63 | 71 | 73 | 80 | 81 | 85 => 1,
        _ => 0,
    }
}

pub fn http() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(USER_AGENT)
            .timeout(REQUEST_TIMEOUT)
            .gzip(true)
            .build()
            .expect("reqwest client should build with default config")
    })
}

pub fn parse_options<T: serde::de::DeserializeOwned>(
    raw: Option<&toml::Value>,
    fetcher: &str,
) -> Result<T, String> {
    match raw {
        None => Err(format!(
            "{fetcher} requires `latitude` and `longitude` options"
        )),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

pub fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}
