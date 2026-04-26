//! `weather` fetcher — Open-Meteo forecast for a fixed (latitude, longitude).
//!
//! Safety::Safe because the host is hardcoded: the user supplies coordinates, not a URL, so
//! config can't redirect traffic to an attacker-controlled origin. No API key is required and
//! no token leaves the machine.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, TextData};
use crate::render::Shape;

use super::github::common::cache_key;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_BASE: &str = "https://api.open-meteo.com/v1/forecast";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "latitude",
        type_hint: "float (degrees)",
        required: true,
        default: None,
        description: "Latitude of the location to query (e.g., `35.68` for Tokyo).",
    },
    OptionSchema {
        name: "longitude",
        type_hint: "float (degrees)",
        required: true,
        default: None,
        description: "Longitude of the location to query (e.g., `139.76` for Tokyo).",
    },
    OptionSchema {
        name: "units",
        type_hint: "\"metric\" | \"imperial\"",
        required: false,
        default: Some("\"metric\""),
        description: "Temperature and wind unit system. Metric renders °C / m/s; imperial renders °F / mph.",
    },
];

/// Open-Meteo forecast widget. Entries shape (primary): condition / temp / wind / humidity.
pub struct WeatherFetcher;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherOptions {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub units: Option<Units>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

#[async_trait]
impl Fetcher for WeatherFetcher {
    fn name(&self) -> &str {
        "weather"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Current-conditions forecast for a fixed (latitude, longitude) via Open-Meteo. `Entries` shows condition / temperature / wind / humidity rows; `Text` collapses them into a single line. Metric or imperial units, no API key required."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Entries, Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        let extra = ctx
            .options
            .as_ref()
            .and_then(|v| toml::to_string(v).ok())
            .unwrap_or_default();
        cache_key(self.name(), ctx, &extra)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Entries => entries(&Sample::default()),
            Shape::Text => Body::Text(TextData {
                value: Sample::default().as_text(),
            }),
            _ => return None,
        })
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: WeatherOptions =
            parse_options(ctx.options.as_ref()).map_err(FetchError::Failed)?;
        let units = opts.units.unwrap_or_default();
        let report = fetch_report(opts.latitude, opts.longitude, units).await?;
        let body = match ctx.shape.unwrap_or(Shape::Entries) {
            Shape::Text => Body::Text(TextData {
                value: report.as_text(),
            }),
            _ => entries(&report),
        };
        Ok(payload(body))
    }
}

async fn fetch_report(lat: f64, lon: f64, units: Units) -> Result<Sample, FetchError> {
    let url = build_url(lat, lon, units);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("weather request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("weather {status}")));
    }
    let raw: ApiResponse = res
        .json()
        .await
        .map_err(|e| FetchError::Failed(format!("weather json parse: {e}")))?;
    Ok(Sample {
        code: raw.current.weather_code,
        temperature: raw.current.temperature_2m,
        wind_speed: raw.current.wind_speed_10m,
        humidity: raw.current.relative_humidity_2m,
        units,
    })
}

fn build_url(lat: f64, lon: f64, units: Units) -> String {
    let base = format!(
        "{API_BASE}?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code,wind_speed_10m,relative_humidity_2m"
    );
    match units {
        Units::Metric => format!("{base}&wind_speed_unit=ms"),
        Units::Imperial => format!("{base}&temperature_unit=fahrenheit&wind_speed_unit=mph"),
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    current: Current,
}

#[derive(Debug, Deserialize)]
struct Current {
    temperature_2m: f64,
    weather_code: u16,
    wind_speed_10m: f64,
    relative_humidity_2m: u8,
}

struct Sample {
    code: u16,
    temperature: f64,
    wind_speed: f64,
    humidity: u8,
    units: Units,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            code: 3,
            temperature: 18.0,
            wind_speed: 4.0,
            humidity: 67,
            units: Units::Metric,
        }
    }
}

impl Sample {
    fn temperature_label(&self) -> String {
        let unit = match self.units {
            Units::Metric => "°C",
            Units::Imperial => "°F",
        };
        format!("{:.0}{unit}", self.temperature)
    }
    fn wind_label(&self) -> String {
        let unit = match self.units {
            Units::Metric => "m/s",
            Units::Imperial => "mph",
        };
        format!("{:.0} {unit}", self.wind_speed)
    }
    fn humidity_label(&self) -> String {
        format!("{}%", self.humidity)
    }
    fn as_text(&self) -> String {
        let (emoji, condition) = weather_description(self.code);
        format!(
            "{emoji} {condition} · {} · 💨 {} · 💧 {}",
            self.temperature_label(),
            self.wind_label(),
            self.humidity_label(),
        )
    }
}

fn entries(s: &Sample) -> Body {
    let (emoji, condition) = weather_description(s.code);
    let rows = [
        (format!("{emoji} {condition}"), s.temperature_label()),
        ("💨 wind".into(), s.wind_label()),
        ("💧 humidity".into(), s.humidity_label()),
    ];
    Body::Entries(EntriesData {
        items: rows
            .into_iter()
            .map(|(k, v)| Entry {
                key: k,
                value: Some(v),
                status: None,
            })
            .collect(),
    })
}

/// WMO weather interpretation codes (Open-Meteo uses the standard table).
fn weather_description(code: u16) -> (&'static str, &'static str) {
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

fn http() -> &'static Client {
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

fn parse_options<T: serde::de::DeserializeOwned>(raw: Option<&toml::Value>) -> Result<T, String> {
    match raw {
        None => Err("weather requires `latitude` and `longitude` options".into()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

fn payload(body: Body) -> Payload {
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
    fn build_url_metric_requests_ms_wind() {
        let url = build_url(35.68, 139.76, Units::Metric);
        assert!(url.contains("latitude=35.68"));
        assert!(url.contains("longitude=139.76"));
        assert!(url.contains("wind_speed_unit=ms"));
        assert!(!url.contains("fahrenheit"));
        assert!(!url.contains("mph"));
    }

    #[test]
    fn build_url_imperial_sets_fahrenheit_and_mph() {
        let url = build_url(40.71, -74.0, Units::Imperial);
        assert!(url.contains("temperature_unit=fahrenheit"));
        assert!(url.contains("wind_speed_unit=mph"));
    }

    #[test]
    fn weather_description_covers_canonical_codes() {
        assert_eq!(weather_description(0).1, "clear");
        assert_eq!(weather_description(3).1, "overcast");
        assert_eq!(weather_description(63).1, "rain");
        assert_eq!(weather_description(95).1, "thunderstorm");
        assert_eq!(weather_description(1234).1, "unknown");
    }

    #[test]
    fn sample_entries_expose_three_rows() {
        let body = entries(&Sample::default());
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 3);
        assert!(e.items[0].value.as_deref().unwrap().ends_with("°C"));
        assert!(e.items[1].value.as_deref().unwrap().ends_with("m/s"));
        assert!(e.items[2].value.as_deref().unwrap().ends_with('%'));
    }

    #[test]
    fn imperial_units_render_fahrenheit_and_mph() {
        let s = Sample {
            units: Units::Imperial,
            ..Sample::default()
        };
        assert!(s.temperature_label().ends_with("°F"));
        assert!(s.wind_label().ends_with("mph"));
    }

    #[test]
    fn api_response_deserializes_open_meteo_current() {
        let raw = r#"{"current":{"time":"2026-04-23T12:00","temperature_2m":18.2,"weather_code":3,"wind_speed_10m":4.1,"relative_humidity_2m":67}}"#;
        let parsed: ApiResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.current.weather_code, 3);
        assert_eq!(parsed.current.relative_humidity_2m, 67);
    }

    #[test]
    fn parse_options_requires_latitude_and_longitude() {
        let err: String = parse_options::<WeatherOptions>(None).unwrap_err();
        assert!(err.contains("latitude"));
    }

    #[test]
    fn parse_options_accepts_floats() {
        let raw: toml::Value = toml::from_str("latitude = 35.68\nlongitude = 139.76").unwrap();
        let opts: WeatherOptions = parse_options(Some(&raw)).unwrap();
        assert_eq!(opts.latitude, 35.68);
        assert_eq!(opts.longitude, 139.76);
    }

    #[test]
    fn parse_options_rejects_unknown_keys() {
        let raw: toml::Value =
            toml::from_str("latitude = 1.0\nlongitude = 2.0\nbogus = true").unwrap();
        assert!(parse_options::<WeatherOptions>(Some(&raw)).is_err());
    }

    /// Live smoke test — hits Open-Meteo. `#[ignore]` keeps CI offline-safe; run with
    /// `cargo test -- --ignored fetcher::weather::tests::live` to verify real API shape.
    #[tokio::test]
    #[ignore]
    async fn live_tokyo_forecast_populates_entries() {
        let report = fetch_report(35.68, 139.76, Units::Metric).await.unwrap();
        let body = entries(&report);
        let Body::Entries(e) = body else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 3);
        for row in &e.items {
            eprintln!("{} → {}", row.key, row.value.as_deref().unwrap_or(""));
        }
    }
}
