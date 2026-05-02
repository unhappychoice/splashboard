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
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, NumberSeriesData, Payload, PointSeries,
    PointSeriesData, Status, TextData,
};
use crate::render::Shape;

use super::github::common::cache_key;
use super::{FetchContext, FetchError, Fetcher, Safety};

const API_BASE: &str = "https://api.open-meteo.com/v1/forecast";
const USER_AGENT: &str = concat!("splashboard/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const HOURLY_HOURS: usize = 24;

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
        "Forecast for a fixed (latitude, longitude) via Open-Meteo. `Entries` / `Text` summarise current conditions; `PointSeries` carries the next 24h of hourly temperature (signed °C / °F); `NumberSeries` / `Bars` carry hourly precipitation in tenths of mm/inch; `Badge` flags severe weather codes (thunderstorm = error, freezing / snow / heavy rain = warn). Metric or imperial units, no API key required."
    }
    fn shapes(&self) -> &[Shape] {
        &[
            Shape::Entries,
            Shape::Text,
            Shape::PointSeries,
            Shape::NumberSeries,
            Shape::Bars,
            Shape::Badge,
        ]
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
        let sample = Sample::default();
        Some(match shape {
            Shape::Entries => entries(&sample),
            Shape::Text => Body::Text(TextData {
                value: sample.as_text(),
            }),
            Shape::PointSeries => point_series(&sample.hourly, sample.units),
            Shape::NumberSeries => number_series(&sample.hourly),
            Shape::Bars => precipitation_bars(&sample.hourly),
            Shape::Badge => Body::Badge(weather_badge(sample.code)),
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
            Shape::PointSeries => point_series(&report.hourly, report.units),
            Shape::NumberSeries => number_series(&report.hourly),
            Shape::Bars => precipitation_bars(&report.hourly),
            Shape::Badge => Body::Badge(weather_badge(report.code)),
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
        hourly: hourly_from(&raw.hourly),
        units,
    })
}

fn hourly_from(raw: &Hourly) -> Vec<HourPoint> {
    let temps = raw.temperature_2m.iter().copied();
    let precs = raw
        .precipitation
        .iter()
        .copied()
        .chain(std::iter::repeat(0.0));
    temps
        .zip(precs)
        .take(HOURLY_HOURS)
        .enumerate()
        .map(|(i, (temp, precip))| HourPoint {
            offset_hours: i as u32,
            temperature: temp,
            precipitation: precip,
        })
        .collect()
}

fn build_url(lat: f64, lon: f64, units: Units) -> String {
    let base = format!(
        "{API_BASE}?latitude={lat}&longitude={lon}\
         &current=temperature_2m,weather_code,wind_speed_10m,relative_humidity_2m\
         &hourly=temperature_2m,precipitation\
         &forecast_days=2"
    );
    match units {
        Units::Metric => format!("{base}&wind_speed_unit=ms"),
        Units::Imperial => format!("{base}&temperature_unit=fahrenheit&wind_speed_unit=mph"),
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    current: Current,
    #[serde(default)]
    hourly: Hourly,
}

#[derive(Debug, Deserialize)]
struct Current {
    temperature_2m: f64,
    weather_code: u16,
    wind_speed_10m: f64,
    relative_humidity_2m: u8,
}

#[derive(Debug, Default, Deserialize)]
struct Hourly {
    #[serde(default)]
    temperature_2m: Vec<f64>,
    #[serde(default)]
    precipitation: Vec<f64>,
}

struct Sample {
    code: u16,
    temperature: f64,
    wind_speed: f64,
    humidity: u8,
    hourly: Vec<HourPoint>,
    units: Units,
}

#[derive(Debug, Clone, Copy)]
struct HourPoint {
    offset_hours: u32,
    temperature: f64,
    /// Precipitation in mm for metric, inches for imperial.
    precipitation: f64,
}

impl Default for Sample {
    fn default() -> Self {
        let hourly = (0..HOURLY_HOURS as u32)
            .map(|i| HourPoint {
                offset_hours: i,
                temperature: 16.0 + ((i as f64) * 0.4 - (i as f64 / 8.0).sin() * 4.0),
                precipitation: if (10..=14).contains(&i) { 0.6 } else { 0.0 },
            })
            .collect();
        Self {
            code: 3,
            temperature: 18.0,
            wind_speed: 4.0,
            humidity: 67,
            hourly,
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

fn point_series(hourly: &[HourPoint], units: Units) -> Body {
    let unit_label = match units {
        Units::Metric => "°C",
        Units::Imperial => "°F",
    };
    Body::PointSeries(PointSeriesData {
        series: vec![PointSeries {
            name: format!("temperature ({unit_label})"),
            points: hourly
                .iter()
                .map(|h| (f64::from(h.offset_hours), h.temperature))
                .collect(),
        }],
    })
}

/// Hourly precipitation as `NumberSeries`. Temperature can go negative and would silently
/// clamp to 0 in `u64`, so the curve lives on `PointSeries` instead. Precipitation is
/// non-negative by definition; values are tenths of a millimetre (or an inch on imperial)
/// so one decimal of precision survives the integer round-trip.
fn number_series(hourly: &[HourPoint]) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: hourly
            .iter()
            .map(|h| (h.precipitation.max(0.0) * 10.0).round() as u64)
            .collect(),
    })
}

fn precipitation_bars(hourly: &[HourPoint]) -> Body {
    Body::Bars(BarsData {
        bars: hourly
            .iter()
            .map(|h| Bar {
                label: format!("+{}h", h.offset_hours),
                // Bars are integer-valued; multiply by 10 to keep one decimal of precision
                // (i.e. units = "tenths of mm" / "tenths of an inch").
                value: (h.precipitation.max(0.0) * 10.0).round() as u64,
            })
            .collect(),
    })
}

fn weather_badge(code: u16) -> BadgeData {
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

    fn entries_data(body: Body) -> Option<EntriesData> {
        match body {
            Body::Entries(data) => Some(data),
            _ => None,
        }
    }

    fn point_series_data(body: Body) -> Option<PointSeriesData> {
        match body {
            Body::PointSeries(data) => Some(data),
            _ => None,
        }
    }

    fn number_series_data(body: Body) -> Option<NumberSeriesData> {
        match body {
            Body::NumberSeries(data) => Some(data),
            _ => None,
        }
    }

    fn bars_data(body: Body) -> Option<BarsData> {
        match body {
            Body::Bars(data) => Some(data),
            _ => None,
        }
    }

    fn text_data(body: Body) -> Option<TextData> {
        match body {
            Body::Text(data) => Some(data),
            _ => None,
        }
    }

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
        assert_eq!(weather_description(56).1, "freezing drizzle");
        assert_eq!(weather_description(63).1, "rain");
        assert_eq!(weather_description(95).1, "thunderstorm");
        assert_eq!(weather_description(1234).1, "unknown");
    }

    #[test]
    fn sample_entries_expose_three_rows() {
        let e = entries_data(entries(&Sample::default())).unwrap();
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

    #[test]
    fn point_series_carries_one_temperature_series_with_24_points() {
        let d = point_series_data(point_series(&Sample::default().hourly, Units::Metric)).unwrap();
        assert_eq!(d.series.len(), 1);
        assert_eq!(d.series[0].points.len(), HOURLY_HOURS);
        assert!(d.series[0].name.contains("°C"));
    }

    #[test]
    fn number_series_carries_precipitation_in_tenths() {
        let hourly = vec![
            HourPoint {
                offset_hours: 0,
                temperature: -3.0, // negative temp must NOT clamp the precip slot to 0
                precipitation: 0.6,
            },
            HourPoint {
                offset_hours: 1,
                temperature: -1.0,
                precipitation: 0.0,
            },
            HourPoint {
                offset_hours: 2,
                temperature: 1.0,
                precipitation: 1.25,
            },
        ];
        let d = number_series_data(number_series(&hourly)).unwrap();
        // 0.6 mm → 6 tenths, 0.0 → 0, 1.25 → 13 (rounded).
        assert_eq!(d.values, vec![6, 0, 13]);
    }

    #[test]
    fn point_series_temperature_preserves_negative_readings() {
        // PointSeries is f64 — make sure sub-zero temps survive end-to-end (the bug fixed by
        // moving temperature off `NumberSeries`, which would have clamped these to 0).
        let hourly = vec![
            HourPoint {
                offset_hours: 0,
                temperature: -5.0,
                precipitation: 0.0,
            },
            HourPoint {
                offset_hours: 1,
                temperature: -2.5,
                precipitation: 0.0,
            },
        ];
        let d = point_series_data(point_series(&hourly, Units::Metric)).unwrap();
        assert_eq!(d.series[0].points[0].1, -5.0);
        assert_eq!(d.series[0].points[1].1, -2.5);
    }

    #[test]
    fn precipitation_bars_are_labelled_with_hour_offsets() {
        let d = bars_data(precipitation_bars(&Sample::default().hourly)).unwrap();
        assert_eq!(d.bars.len(), HOURLY_HOURS);
        assert_eq!(d.bars[0].label, "+0h");
        assert_eq!(d.bars[5].label, "+5h");
    }

    #[test]
    fn weather_badge_severity_follows_wmo_groups() {
        assert_eq!(weather_badge(0).status, Status::Ok);
        assert_eq!(weather_badge(63).status, Status::Warn);
        assert_eq!(weather_badge(75).status, Status::Warn);
        assert_eq!(weather_badge(95).status, Status::Error);
    }

    #[test]
    fn fetcher_metadata_cache_key_and_samples_cover_supported_shapes() {
        let fetcher = WeatherFetcher;
        let ctx = FetchContext {
            widget_id: "weather".into(),
            timeout: std::time::Duration::from_secs(1),
            shape: Some(Shape::Entries),
            ..Default::default()
        };
        let with_options = FetchContext {
            options: Some(
                toml::from_str("latitude = 35.68\nlongitude = 139.76\nunits = \"imperial\"")
                    .unwrap(),
            ),
            ..ctx.clone()
        };
        assert_eq!(fetcher.name(), "weather");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Open-Meteo"));
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            vec!["latitude", "longitude", "units"]
        );
        assert_eq!(
            fetcher.shapes(),
            [
                Shape::Entries,
                Shape::Text,
                Shape::PointSeries,
                Shape::NumberSeries,
                Shape::Bars,
                Shape::Badge,
            ]
            .as_slice()
        );
        assert_ne!(fetcher.cache_key(&ctx), fetcher.cache_key(&with_options));
        for &shape in fetcher.shapes() {
            let body = fetcher.sample_body(shape).unwrap();
            assert!(matches!(
                (shape, body),
                (Shape::Entries, Body::Entries(_))
                    | (Shape::Text, Body::Text(_))
                    | (Shape::PointSeries, Body::PointSeries(_))
                    | (Shape::NumberSeries, Body::NumberSeries(_))
                    | (Shape::Bars, Body::Bars(_))
                    | (Shape::Badge, Body::Badge(_))
            ));
        }
        let text = fetcher
            .sample_body(Shape::Text)
            .and_then(text_data)
            .unwrap();
        let series = fetcher
            .sample_body(Shape::PointSeries)
            .and_then(point_series_data)
            .unwrap();
        assert!(text.value.contains("💨"));
        assert_eq!(series.series[0].name, "temperature (°C)");
        assert!(fetcher.sample_body(Shape::Calendar).is_none());
    }

    #[test]
    fn helper_paths_cover_padding_statuses_and_singletons() {
        let points = hourly_from(&Hourly {
            temperature_2m: (0..30).map(|i| i as f64).collect(),
            precipitation: vec![0.5],
        });
        let series = point_series_data(point_series(&points[..2], Units::Imperial)).unwrap();
        let text = text_data(payload(Body::Text(TextData { value: "ok".into() })).body).unwrap();
        assert_eq!(points.len(), HOURLY_HOURS);
        assert_eq!(points[1].precipitation, 0.0);
        assert_eq!(points[23].offset_hours, 23);
        assert_eq!(series.series[0].name, "temperature (°F)");
        assert_eq!(text.value, "ok");
        assert!(std::ptr::eq(http(), http()));
        assert_eq!(weather_badge(56).label, "freezing");
        assert_eq!(weather_badge(96).label, "thunderstorm");
        assert_eq!(
            [1, 2, 45, 51, 66, 71, 81, 86, 96].map(|code| weather_description(code).1),
            [
                "mostly clear",
                "partly cloudy",
                "fog",
                "drizzle",
                "freezing rain",
                "snow",
                "rain showers",
                "snow showers",
                "thunderstorm w/ hail",
            ]
        );
    }

    #[test]
    fn body_extractors_return_none_for_wrong_variants() {
        assert!(entries_data(Body::Text(TextData { value: "x".into() })).is_none());
        assert!(point_series_data(entries(&Sample::default())).is_none());
        assert!(number_series_data(Body::Bars(BarsData { bars: vec![] })).is_none());
        assert!(bars_data(Body::NumberSeries(NumberSeriesData { values: vec![] })).is_none());
        assert!(text_data(Body::Badge(weather_badge(0))).is_none());
    }

    #[tokio::test]
    async fn fetch_rejects_missing_options_before_network() {
        let ctx = FetchContext {
            widget_id: "weather".into(),
            timeout: std::time::Duration::from_secs(1),
            shape: Some(Shape::Text),
            ..Default::default()
        };
        let err = WeatherFetcher.fetch(&ctx).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("latitude")));
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
