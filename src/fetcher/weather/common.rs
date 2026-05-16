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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_labels_cover_both_systems() {
        assert_eq!(Units::Metric.temperature_label(), "°C");
        assert_eq!(Units::Imperial.temperature_label(), "°F");
        assert_eq!(Units::Metric.wind_label(), "m/s");
        assert_eq!(Units::Imperial.wind_label(), "mph");
        assert_eq!(Units::Metric.precipitation_label(), "mm");
        assert_eq!(Units::Imperial.precipitation_label(), "in");
    }

    #[test]
    fn units_default_is_metric() {
        assert_eq!(Units::default(), Units::Metric);
    }

    #[test]
    fn weather_description_covers_every_wmo_bucket() {
        // One representative code per match arm — make sure every branch is exercised so
        // future edits to the table can't silently drop an icon/label pair.
        for (code, expected_label) in [
            (0u16, "clear"),
            (1, "mostly clear"),
            (2, "partly cloudy"),
            (3, "overcast"),
            (45, "fog"),
            (48, "fog"),
            (51, "drizzle"),
            (53, "drizzle"),
            (55, "drizzle"),
            (56, "freezing drizzle"),
            (57, "freezing drizzle"),
            (61, "rain"),
            (63, "rain"),
            (65, "rain"),
            (66, "freezing rain"),
            (67, "freezing rain"),
            (71, "snow"),
            (73, "snow"),
            (75, "snow"),
            (77, "snow"),
            (80, "rain showers"),
            (81, "rain showers"),
            (82, "rain showers"),
            (85, "snow showers"),
            (86, "snow showers"),
            (95, "thunderstorm"),
            (96, "thunderstorm w/ hail"),
            (99, "thunderstorm w/ hail"),
        ] {
            let (icon, label) = weather_description(code);
            assert_eq!(label, expected_label, "label for WMO code {code}");
            assert!(
                !icon.is_empty(),
                "expected a non-empty icon for WMO code {code}"
            );
        }
    }

    #[test]
    fn weather_description_falls_back_for_unknown_code() {
        assert_eq!(weather_description(7777), ("🌡", "unknown"));
    }

    #[test]
    fn weather_badge_classifies_each_severity_bucket() {
        // Thunderstorm → Error tier.
        for code in [95u16, 96, 99] {
            let b = weather_badge(code);
            assert_eq!(b.status, Status::Error, "WMO {code} should be Error");
        }
        // Freezing precip → Warn with the "freezing" label.
        for code in [56u16, 57, 66, 67] {
            let b = weather_badge(code);
            assert_eq!(b.status, Status::Warn);
            assert_eq!(b.label, "freezing", "WMO {code} should be freezing-tier");
        }
        // Heavy precip → Warn with the "heavy precip" label.
        for code in [65u16, 75, 82, 86] {
            let b = weather_badge(code);
            assert_eq!(b.status, Status::Warn);
            assert_eq!(b.label, "heavy precip", "WMO {code} should be heavy-precip");
        }
        // Ordinary precip → Warn with the plain "precip" label.
        for code in [61u16, 63, 71, 73, 80, 81, 85] {
            let b = weather_badge(code);
            assert_eq!(b.status, Status::Warn);
            assert_eq!(b.label, "precip", "WMO {code} should be precip-tier");
        }
        // Fair weather falls back to the description text under Status::Ok.
        let clear = weather_badge(0);
        assert_eq!(clear.status, Status::Ok);
        assert_eq!(clear.label, "clear");
    }

    #[test]
    fn severity_rank_buckets_match_weather_badge() {
        assert_eq!(severity_rank(95), 3);
        assert_eq!(severity_rank(96), 3);
        assert_eq!(severity_rank(99), 3);
        for c in [56u16, 57, 65, 66, 67, 75, 82, 86] {
            assert_eq!(severity_rank(c), 2, "WMO {c} should rank 2");
        }
        for c in [61u16, 63, 71, 73, 80, 81, 85] {
            assert_eq!(severity_rank(c), 1, "WMO {c} should rank 1");
        }
        assert_eq!(severity_rank(0), 0);
        assert_eq!(severity_rank(3), 0);
        assert_eq!(severity_rank(7777), 0);
    }

    #[test]
    fn http_returns_the_shared_client_instance() {
        // Build twice and confirm we get the same singleton back — covers the
        // OnceLock init path and the cached-read path on the second call.
        let a = http() as *const Client;
        let b = http() as *const Client;
        assert_eq!(a, b);
    }

    #[test]
    fn parse_options_missing_options_surfaces_fetcher_name() {
        let err = parse_options::<Units>(None, "weather_now").unwrap_err();
        assert!(err.contains("weather_now"));
        assert!(err.contains("latitude"));
    }

    #[test]
    fn parse_options_invalid_payload_prefixes_invalid_options() {
        // `Units` deserializes from a bare lowercase string, so a table fails the conversion.
        let raw: toml::Value = toml::from_str("foo = 1").unwrap();
        let err = parse_options::<Units>(Some(&raw), "weather_now").unwrap_err();
        assert!(err.starts_with("invalid options:"), "got: {err}");
    }

    #[test]
    fn parse_options_accepts_valid_payload() {
        let raw = toml::Value::String("imperial".into());
        let units: Units = parse_options(Some(&raw), "weather_now").unwrap();
        assert_eq!(units, Units::Imperial);
    }

    #[test]
    fn payload_wraps_body_with_empty_metadata() {
        let p = payload(Body::Text(crate::payload::TextData { value: "x".into() }));
        assert!(p.icon.is_none());
        assert!(p.status.is_none());
        assert!(p.format.is_none());
        match p.body {
            Body::Text(t) => assert_eq!(t.value, "x"),
            _ => panic!("expected Text body"),
        }
    }
}
