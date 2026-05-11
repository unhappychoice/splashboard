//! `weather_forecast` fetcher — Open-Meteo multi-day daily forecast for a fixed
//! (latitude, longitude). Sibling of `weather` which covers current conditions only.
//!
//! Safety::Safe — host is hardcoded; config supplies coordinates / units / day count, never
//! a URL. No API key.

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Weekday};
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Bar, BarsData, Body, EntriesData, Entry, NumberSeriesData, Payload, PointSeries,
    PointSeriesData, RatioData, Status, TextBlockData, TextData, TimelineData, TimelineEvent,
};
use crate::render::Shape;

use super::super::github::common::cache_key;
use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::common::{
    API_BASE, Units, http, parse_options, payload, severity_rank, weather_badge,
    weather_description,
};

const DEFAULT_DAYS: u8 = 3;
const MIN_DAYS: u8 = 1;
const MAX_DAYS: u8 = 7;

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
        description: "Temperature unit system. Metric renders °C; imperial renders °F and reports precipitation in inches.",
    },
    OptionSchema {
        name: "days",
        type_hint: "int (1..=7)",
        required: false,
        default: Some("3"),
        description: "How many days of forecast to fetch. Clamped to 1..=7 — Open-Meteo serves longer windows, but accuracy drops past a week.",
    },
];

pub struct WeatherForecastFetcher;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherForecastOptions {
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub units: Option<Units>,
    #[serde(default)]
    pub days: Option<u8>,
}

#[async_trait]
impl Fetcher for WeatherForecastFetcher {
    fn name(&self) -> &str {
        "weather_forecast"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Daily multi-day forecast for a fixed (latitude, longitude) via Open-Meteo. `TextBlock` / `Entries` / `Text` summarise highs / lows / precipitation per day; `Ratio` reports the worst precipitation probability in the window; `NumberSeries` carries per-day rainfall totals (tenths of mm or inch) for sparkline / histogram consumers; `Bars` carries per-day precipitation **probability** (%) so the bar chart stays informative even in dry forecasts; `PointSeries` carries high+low temperature curves across days; `Badge` flags the worst weather code; `Timeline` lays the days out chronologically. `days` defaults to 3 (range 1..=7), metric units by default, no API key required."
    }
    fn shapes(&self) -> &[Shape] {
        &[
            Shape::TextBlock,
            Shape::Text,
            Shape::Entries,
            Shape::Ratio,
            Shape::NumberSeries,
            Shape::PointSeries,
            Shape::Bars,
            Shape::Badge,
            Shape::Timeline,
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
        Some(body_for_shape(&Forecast::sample(), shape))
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: WeatherForecastOptions =
            parse_options(ctx.options.as_ref(), self.name()).map_err(FetchError::Failed)?;
        let days = resolve_days(opts.days).map_err(FetchError::Failed)?;
        let units = opts.units.unwrap_or_default();
        let forecast = fetch_forecast(opts.latitude, opts.longitude, units, days).await?;
        let shape = ctx.shape.unwrap_or(Shape::TextBlock);
        Ok(payload(body_for_shape(&forecast, shape)))
    }
}

fn body_for_shape(forecast: &Forecast, shape: Shape) -> Body {
    match shape {
        Shape::Text => text(forecast),
        Shape::Entries => entries(forecast),
        Shape::Ratio => ratio(forecast),
        Shape::NumberSeries => number_series(forecast),
        Shape::PointSeries => point_series(forecast),
        Shape::Bars => bars(forecast),
        Shape::Badge => badge(forecast),
        Shape::Timeline => timeline(forecast),
        _ => text_block(forecast),
    }
}

fn resolve_days(raw: Option<u8>) -> Result<u8, String> {
    let value = raw.unwrap_or(DEFAULT_DAYS);
    if !(MIN_DAYS..=MAX_DAYS).contains(&value) {
        return Err(format!(
            "weather_forecast `days` must be between {MIN_DAYS} and {MAX_DAYS}, got {value}"
        ));
    }
    Ok(value)
}

async fn fetch_forecast(
    lat: f64,
    lon: f64,
    units: Units,
    days: u8,
) -> Result<Forecast, FetchError> {
    let url = build_url(lat, lon, units, days);
    let res = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("weather_forecast request failed: {e}")))?;
    let status = res.status();
    if !status.is_success() {
        return Err(FetchError::Failed(format!("weather_forecast {status}")));
    }
    let raw: ApiResponse = res
        .json()
        .await
        .map_err(|e| FetchError::Failed(format!("weather_forecast json parse: {e}")))?;
    Ok(Forecast {
        days: days_from(&raw.daily),
        units,
    })
}

fn build_url(lat: f64, lon: f64, units: Units, days: u8) -> String {
    let base = format!(
        "{API_BASE}?latitude={lat}&longitude={lon}\
         &daily=temperature_2m_max,temperature_2m_min,weather_code,precipitation_sum,precipitation_probability_max\
         &forecast_days={days}\
         &timezone=auto"
    );
    match units {
        Units::Metric => base,
        Units::Imperial => format!("{base}&temperature_unit=fahrenheit&precipitation_unit=inch"),
    }
}

fn days_from(raw: &Daily) -> Vec<DayPoint> {
    let len = raw
        .time
        .len()
        .min(raw.temperature_2m_max.len())
        .min(raw.temperature_2m_min.len())
        .min(raw.weather_code.len());
    (0..len)
        .filter_map(|i| {
            let date = NaiveDate::parse_from_str(&raw.time[i], "%Y-%m-%d").ok()?;
            Some(DayPoint {
                date,
                weather_code: raw.weather_code[i],
                high: raw.temperature_2m_max[i],
                low: raw.temperature_2m_min[i],
                precipitation: raw.precipitation_sum.get(i).copied().unwrap_or(0.0),
                precip_probability: raw
                    .precipitation_probability_max
                    .get(i)
                    .copied()
                    .unwrap_or(0),
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    #[serde(default)]
    daily: Daily,
}

#[derive(Debug, Default, Deserialize)]
struct Daily {
    #[serde(default)]
    time: Vec<String>,
    #[serde(default)]
    temperature_2m_max: Vec<f64>,
    #[serde(default)]
    temperature_2m_min: Vec<f64>,
    #[serde(default)]
    weather_code: Vec<u16>,
    #[serde(default)]
    precipitation_sum: Vec<f64>,
    #[serde(default)]
    precipitation_probability_max: Vec<u8>,
}

struct Forecast {
    days: Vec<DayPoint>,
    units: Units,
}

#[derive(Debug, Clone, Copy)]
struct DayPoint {
    date: NaiveDate,
    weather_code: u16,
    high: f64,
    low: f64,
    precipitation: f64,
    precip_probability: u8,
}

impl Forecast {
    fn sample() -> Self {
        let base = NaiveDate::from_ymd_opt(2026, 5, 11).expect("valid date");
        let days = [
            (2u16, 22.4, 15.1, 0.0, 10u8),
            (63, 18.2, 12.8, 8.4, 80),
            (1, 20.5, 14.7, 0.0, 30),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (code, high, low, precip, prob))| DayPoint {
            date: base + chrono::Duration::days(i as i64),
            weather_code: code,
            high,
            low,
            precipitation: precip,
            precip_probability: prob,
        })
        .collect();
        Self {
            days,
            units: Units::Metric,
        }
    }
}

fn weekday_short(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn temp_range_label(day: &DayPoint, units: Units) -> String {
    format!(
        "{:.0}°/{:.0}°{}",
        day.high,
        day.low,
        units.temperature_label().trim_start_matches('°')
    )
}

fn precip_label(day: &DayPoint, units: Units) -> Option<String> {
    if day.precipitation <= 0.0 {
        return None;
    }
    let precision = match units {
        Units::Metric => 1,
        Units::Imperial => 2,
    };
    Some(format!(
        "{:.*}{}",
        precision,
        day.precipitation,
        units.precipitation_label()
    ))
}

fn precip_summary(day: &DayPoint, units: Units) -> String {
    match precip_label(day, units) {
        Some(amount) => format!("💧{}% {amount}", day.precip_probability),
        None => format!("💧{}%", day.precip_probability),
    }
}

fn day_line(day: &DayPoint, units: Units) -> String {
    let (emoji, _) = weather_description(day.weather_code);
    format!(
        "{}  {emoji} {}  {}",
        weekday_short(day.date.weekday()),
        temp_range_label(day, units),
        precip_summary(day, units)
    )
}

fn text_block(forecast: &Forecast) -> Body {
    Body::TextBlock(TextBlockData {
        lines: forecast
            .days
            .iter()
            .map(|d| day_line(d, forecast.units))
            .collect(),
    })
}

fn text(forecast: &Forecast) -> Body {
    let summary = forecast
        .days
        .iter()
        .map(|d| {
            let (emoji, _) = weather_description(d.weather_code);
            format!("{emoji} {}", temp_range_label(d, forecast.units))
        })
        .collect::<Vec<_>>()
        .join(" → ");
    Body::Text(TextData { value: summary })
}

fn entries(forecast: &Forecast) -> Body {
    Body::Entries(EntriesData {
        items: forecast
            .days
            .iter()
            .map(|d| {
                let (emoji, _) = weather_description(d.weather_code);
                Entry {
                    key: format!("{} {emoji}", weekday_short(d.date.weekday())),
                    value: Some(format!(
                        "{}  {}",
                        temp_range_label(d, forecast.units),
                        precip_summary(d, forecast.units)
                    )),
                    status: None,
                }
            })
            .collect(),
    })
}

/// `Ratio` summarises the worst precipitation chance across the window. Pairs with
/// `gauge_circle` / `gauge_battery` for a "umbrella?" glance — the single highest probability
/// in the next few days, labelled with the day it falls on.
fn ratio(forecast: &Forecast) -> Body {
    let worst = forecast
        .days
        .iter()
        .max_by_key(|d| d.precip_probability)
        .copied();
    let (value, label) = match worst {
        Some(day) => (
            f64::from(day.precip_probability) / 100.0,
            Some(format!("{} rain risk", weekday_short(day.date.weekday()))),
        ),
        None => (0.0, None),
    };
    Body::Ratio(RatioData {
        value,
        label,
        denominator: Some(100),
    })
}

/// Per-day precipitation totals in tenths of mm (or inch for imperial). Same precision
/// strategy as `weather`'s hourly precipitation — survives the `u64` round-trip without
/// losing the sub-millimetre signal that distinguishes drizzle from "actually wet".
fn number_series(forecast: &Forecast) -> Body {
    Body::NumberSeries(NumberSeriesData {
        values: forecast
            .days
            .iter()
            .map(|d| (d.precipitation.max(0.0) * 10.0).round() as u64)
            .collect(),
    })
}

/// Two series — `high` and `low` temperatures — across days. `PointSeries` because
/// temperature can go negative; `NumberSeries` would silently clamp.
fn point_series(forecast: &Forecast) -> Body {
    let unit = forecast.units.temperature_label();
    let to_points = |pick: fn(&DayPoint) -> f64| -> Vec<(f64, f64)> {
        forecast
            .days
            .iter()
            .enumerate()
            .map(|(i, d)| (i as f64, pick(d)))
            .collect()
    };
    Body::PointSeries(PointSeriesData {
        series: vec![
            PointSeries {
                name: format!("high ({unit})"),
                points: to_points(|d| d.high),
            },
            PointSeries {
                name: format!("low ({unit})"),
                points: to_points(|d| d.low),
            },
        ],
    })
}

/// Per-day precipitation **probability** (0..=100) with weekday labels. Probability rather
/// than rainfall amount because most forecasts have wide stretches of `precipitation_sum =
/// 0mm` even when chance-of-rain varies meaningfully (15% Tue, 35% Wed, 10% Thu). The
/// amount sequence stays available on `NumberSeries` for sparkline / histogram consumers.
fn bars(forecast: &Forecast) -> Body {
    Body::Bars(BarsData {
        bars: forecast
            .days
            .iter()
            .map(|d| Bar {
                label: weekday_short(d.date.weekday()).into(),
                value: u64::from(d.precip_probability),
            })
            .collect(),
    })
}

fn badge(forecast: &Forecast) -> Body {
    let worst_code = forecast
        .days
        .iter()
        .max_by_key(|d| severity_rank(d.weather_code))
        .map(|d| d.weather_code)
        .unwrap_or(0);
    let mut data = weather_badge(worst_code);
    if data.status == Status::Ok {
        data = BadgeData {
            status: Status::Ok,
            label: "clear ahead".into(),
        };
    }
    Body::Badge(data)
}

fn timeline(forecast: &Forecast) -> Body {
    Body::Timeline(TimelineData {
        events: forecast
            .days
            .iter()
            .map(|d| {
                let (emoji, condition) = weather_description(d.weather_code);
                TimelineEvent {
                    timestamp: d
                        .date
                        .and_hms_opt(0, 0, 0)
                        .expect("midnight is always valid")
                        .and_utc()
                        .timestamp(),
                    title: format!("{emoji} {condition}"),
                    detail: Some(format!(
                        "{}  {}",
                        temp_range_label(d, forecast.units),
                        precip_summary(d, forecast.units)
                    )),
                    status: Some(weather_badge(d.weather_code).status),
                }
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forecast_with(days: Vec<DayPoint>, units: Units) -> Forecast {
        Forecast { days, units }
    }

    fn day(code: u16, high: f64, low: f64, precip: f64, prob: u8, date: NaiveDate) -> DayPoint {
        DayPoint {
            date,
            weather_code: code,
            high,
            low,
            precipitation: precip,
            precip_probability: prob,
        }
    }

    #[test]
    fn build_url_metric_uses_timezone_auto_and_forecast_days() {
        let url = build_url(35.68, 139.76, Units::Metric, 3);
        assert!(url.contains("latitude=35.68"));
        assert!(url.contains("longitude=139.76"));
        assert!(url.contains("forecast_days=3"));
        assert!(url.contains("timezone=auto"));
        assert!(url.contains("temperature_2m_max"));
        assert!(url.contains("precipitation_probability_max"));
        assert!(!url.contains("fahrenheit"));
        assert!(!url.contains("precipitation_unit=inch"));
    }

    #[test]
    fn build_url_imperial_switches_temperature_and_precipitation_units() {
        let url = build_url(40.71, -74.0, Units::Imperial, 5);
        assert!(url.contains("forecast_days=5"));
        assert!(url.contains("temperature_unit=fahrenheit"));
        assert!(url.contains("precipitation_unit=inch"));
    }

    #[test]
    fn resolve_days_defaults_to_three() {
        assert_eq!(resolve_days(None).unwrap(), DEFAULT_DAYS);
    }

    #[test]
    fn resolve_days_accepts_bounds() {
        assert_eq!(resolve_days(Some(1)).unwrap(), 1);
        assert_eq!(resolve_days(Some(7)).unwrap(), 7);
    }

    #[test]
    fn resolve_days_rejects_out_of_range() {
        assert!(resolve_days(Some(0)).is_err());
        assert!(resolve_days(Some(8)).is_err());
    }

    #[test]
    fn parse_options_requires_coordinates() {
        let err: String = parse_options::<WeatherForecastOptions>(None, "weather_forecast")
            .expect_err("missing options must fail");
        assert!(err.contains("latitude"));
    }

    #[test]
    fn parse_options_accepts_full_form() {
        let raw: toml::Value =
            toml::from_str("latitude = 35.68\nlongitude = 139.76\nunits = \"imperial\"\ndays = 5")
                .unwrap();
        let opts: WeatherForecastOptions = parse_options(Some(&raw), "weather_forecast").unwrap();
        assert_eq!(opts.units, Some(Units::Imperial));
        assert_eq!(opts.days, Some(5));
    }

    #[test]
    fn parse_options_rejects_unknown_keys() {
        let raw: toml::Value =
            toml::from_str("latitude = 1.0\nlongitude = 2.0\nbogus = true").unwrap();
        assert!(parse_options::<WeatherForecastOptions>(Some(&raw), "weather_forecast").is_err());
    }

    #[test]
    fn daily_deserializes_open_meteo_payload() {
        let raw = r#"{"daily":{"time":["2026-05-12","2026-05-13"],"temperature_2m_max":[22.4,18.2],"temperature_2m_min":[15.1,12.8],"weather_code":[2,63],"precipitation_sum":[0.0,8.4],"precipitation_probability_max":[10,80]}}"#;
        let parsed: ApiResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.daily.time.len(), 2);
        assert_eq!(parsed.daily.weather_code, vec![2, 63]);
        let days = days_from(&parsed.daily);
        assert_eq!(days.len(), 2);
        assert_eq!(days[1].weather_code, 63);
        assert_eq!(days[1].precip_probability, 80);
    }

    #[test]
    fn days_from_skips_invalid_dates() {
        let daily = Daily {
            time: vec!["2026-05-12".into(), "not-a-date".into()],
            temperature_2m_max: vec![22.0, 18.0],
            temperature_2m_min: vec![15.0, 12.0],
            weather_code: vec![2, 63],
            precipitation_sum: vec![],
            precipitation_probability_max: vec![],
        };
        let days = days_from(&daily);
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].precipitation, 0.0);
        assert_eq!(days[0].precip_probability, 0);
    }

    #[test]
    fn text_block_emits_one_line_per_day() {
        let forecast = Forecast::sample();
        let Body::TextBlock(data) = text_block(&forecast) else {
            panic!("expected text block");
        };
        assert_eq!(data.lines.len(), 3);
        assert!(data.lines[0].starts_with("Mon  "));
        assert!(data.lines[0].contains("22°/15°"));
        assert!(data.lines[1].contains("💧80%"));
        // Tue has 8.4mm precip → label includes the amount.
        assert!(
            data.lines[1].ends_with("8.4mm"),
            "expected precip amount on rainy day: {:?}",
            data.lines[1]
        );
        // Mon and Wed are dry → no mm suffix.
        assert!(!data.lines[0].contains("mm"));
        assert!(!data.lines[2].contains("mm"));
    }

    #[test]
    fn precip_label_formats_metric_and_imperial_with_appropriate_precision() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
        let wet = day(63, 18.0, 12.0, 8.4, 80, date);
        let dry = day(0, 22.0, 16.0, 0.0, 10, date);
        assert_eq!(precip_label(&wet, Units::Metric), Some("8.4mm".into()));
        assert_eq!(precip_label(&wet, Units::Imperial), Some("8.40in".into()));
        assert_eq!(precip_label(&dry, Units::Metric), None);
    }

    #[test]
    fn text_emits_arrow_joined_summary() {
        let Body::Text(data) = text(&Forecast::sample()) else {
            panic!("expected text");
        };
        assert!(data.value.contains("→"));
        assert_eq!(data.value.matches('→').count(), 2);
        assert!(data.value.contains("22°/15°"));
    }

    #[test]
    fn entries_emit_one_row_per_day_with_weekday_key() {
        let Body::Entries(data) = entries(&Forecast::sample()) else {
            panic!("expected entries");
        };
        assert_eq!(data.items.len(), 3);
        assert!(data.items[0].key.starts_with("Mon "));
        assert!(data.items[0].value.as_deref().unwrap().contains("💧10%"));
    }

    #[test]
    fn ratio_picks_max_precip_probability_and_names_the_day() {
        let Body::Ratio(data) = ratio(&Forecast::sample()) else {
            panic!("expected ratio");
        };
        assert!((data.value - 0.80).abs() < 1e-9);
        assert_eq!(data.denominator, Some(100));
        assert!(data.label.unwrap().starts_with("Tue "));
    }

    #[test]
    fn number_series_holds_per_day_precipitation_in_tenths() {
        let Body::NumberSeries(data) = number_series(&Forecast::sample()) else {
            panic!("expected number series");
        };
        assert_eq!(data.values, vec![0, 84, 0]);
    }

    #[test]
    fn point_series_carries_high_and_low_with_unit_labels() {
        let Body::PointSeries(data) = point_series(&Forecast::sample()) else {
            panic!("expected point series");
        };
        assert_eq!(data.series.len(), 2);
        assert_eq!(data.series[0].name, "high (°C)");
        assert_eq!(data.series[1].name, "low (°C)");
        assert_eq!(data.series[0].points.len(), 3);
        assert_eq!(data.series[0].points[1], (1.0, 18.2));
        assert_eq!(data.series[1].points[2], (2.0, 14.7));
    }

    #[test]
    fn point_series_preserves_negative_temperatures() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let forecast = forecast_with(vec![day(71, -2.0, -8.5, 0.4, 60, date)], Units::Metric);
        let Body::PointSeries(data) = point_series(&forecast) else {
            panic!("expected point series");
        };
        assert_eq!(data.series[0].points[0].1, -2.0);
        assert_eq!(data.series[1].points[0].1, -8.5);
    }

    #[test]
    fn bars_carry_per_day_precipitation_probability_with_weekday_labels() {
        let Body::Bars(data) = bars(&Forecast::sample()) else {
            panic!("expected bars");
        };
        assert_eq!(data.bars.len(), 3);
        assert_eq!(data.bars[0].label, "Mon");
        // Sample probabilities: Mon=10, Tue=80, Wed=30. Bars now expose probability so the
        // chart stays informative in dry forecasts where `precipitation_sum` is all zero.
        assert_eq!(data.bars[0].value, 10);
        assert_eq!(data.bars[1].value, 80);
        assert_eq!(data.bars[2].value, 30);
    }

    #[test]
    fn badge_picks_worst_severity_across_window() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
        let forecast = forecast_with(
            vec![
                day(0, 22.0, 15.0, 0.0, 5, date),
                day(63, 18.0, 12.0, 6.0, 70, date + chrono::Duration::days(1)),
                day(95, 25.0, 17.0, 12.0, 90, date + chrono::Duration::days(2)),
            ],
            Units::Metric,
        );
        let Body::Badge(data) = badge(&forecast) else {
            panic!("expected badge");
        };
        assert_eq!(data.status, Status::Error);
        assert_eq!(data.label, "thunderstorm");
    }

    #[test]
    fn badge_renames_all_clear_label_to_clear_ahead() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
        let forecast = forecast_with(
            vec![
                day(0, 22.0, 15.0, 0.0, 0, date),
                day(1, 23.0, 16.0, 0.0, 5, date + chrono::Duration::days(1)),
            ],
            Units::Metric,
        );
        let Body::Badge(data) = badge(&forecast) else {
            panic!("expected badge");
        };
        assert_eq!(data.status, Status::Ok);
        assert_eq!(data.label, "clear ahead");
    }

    #[test]
    fn timeline_carries_one_event_per_day_with_midnight_utc_timestamps() {
        let Body::Timeline(data) = timeline(&Forecast::sample()) else {
            panic!("expected timeline");
        };
        assert_eq!(data.events.len(), 3);
        // 2026-05-11 midnight UTC.
        assert_eq!(data.events[0].timestamp, 1_778_457_600);
        assert_eq!(data.events[1].timestamp - data.events[0].timestamp, 86_400);
        assert!(data.events[0].title.contains("partly cloudy"));
        assert_eq!(data.events[1].status, Some(Status::Warn));
    }

    #[test]
    fn imperial_units_label_temperatures_in_fahrenheit() {
        let forecast = Forecast {
            units: Units::Imperial,
            ..Forecast::sample()
        };
        let Body::PointSeries(data) = point_series(&forecast) else {
            panic!("expected point series");
        };
        assert_eq!(data.series[0].name, "high (°F)");
        let Body::TextBlock(block) = text_block(&forecast) else {
            panic!("expected text block");
        };
        assert!(block.lines[0].contains("22°/15°F"));
    }

    #[test]
    fn single_day_window_collapses_all_shapes_cleanly() {
        let date = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
        let forecast = forecast_with(vec![day(2, 22.0, 15.0, 0.5, 20, date)], Units::Metric);
        let Body::TextBlock(tb) = text_block(&forecast) else {
            panic!("text block");
        };
        let Body::Text(t) = text(&forecast) else {
            panic!("text");
        };
        let Body::Bars(b) = bars(&forecast) else {
            panic!("bars");
        };
        assert_eq!(tb.lines.len(), 1);
        assert!(!t.value.contains('→'));
        assert_eq!(b.bars.len(), 1);
    }

    #[test]
    fn fetcher_metadata_cache_key_and_samples_cover_supported_shapes() {
        let fetcher = WeatherForecastFetcher;
        let ctx = FetchContext {
            widget_id: "weather_forecast".into(),
            timeout: std::time::Duration::from_secs(1),
            shape: Some(Shape::TextBlock),
            ..Default::default()
        };
        let with_options = FetchContext {
            options: Some(
                toml::from_str("latitude = 35.68\nlongitude = 139.76\ndays = 5").unwrap(),
            ),
            ..ctx.clone()
        };
        assert_eq!(fetcher.name(), "weather_forecast");
        assert_eq!(fetcher.safety(), Safety::Safe);
        assert!(fetcher.description().contains("Open-Meteo"));
        assert_eq!(
            fetcher
                .option_schemas()
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>(),
            vec!["latitude", "longitude", "units", "days"]
        );
        assert_eq!(fetcher.default_shape(), Shape::TextBlock);
        assert_ne!(fetcher.cache_key(&ctx), fetcher.cache_key(&with_options));
        for &shape in fetcher.shapes() {
            let body = fetcher
                .sample_body(shape)
                .expect("sample for declared shape");
            let ok = matches!(
                (shape, &body),
                (Shape::TextBlock, Body::TextBlock(_))
                    | (Shape::Text, Body::Text(_))
                    | (Shape::Entries, Body::Entries(_))
                    | (Shape::Ratio, Body::Ratio(_))
                    | (Shape::NumberSeries, Body::NumberSeries(_))
                    | (Shape::PointSeries, Body::PointSeries(_))
                    | (Shape::Bars, Body::Bars(_))
                    | (Shape::Badge, Body::Badge(_))
                    | (Shape::Timeline, Body::Timeline(_))
            );
            assert!(ok, "shape {shape:?} produced wrong body variant");
        }
    }

    #[tokio::test]
    async fn fetch_rejects_missing_options_before_network() {
        let ctx = FetchContext {
            widget_id: "weather_forecast".into(),
            timeout: std::time::Duration::from_secs(1),
            shape: Some(Shape::TextBlock),
            ..Default::default()
        };
        let err = WeatherForecastFetcher.fetch(&ctx).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("latitude")));
    }

    #[tokio::test]
    async fn fetch_rejects_out_of_range_days_before_network() {
        let ctx = FetchContext {
            widget_id: "weather_forecast".into(),
            timeout: std::time::Duration::from_secs(1),
            shape: Some(Shape::TextBlock),
            options: Some(
                toml::from_str("latitude = 35.68\nlongitude = 139.76\ndays = 30").unwrap(),
            ),
            ..Default::default()
        };
        let err = WeatherForecastFetcher.fetch(&ctx).await.unwrap_err();
        assert!(matches!(err, FetchError::Failed(msg) if msg.contains("days")));
    }

    /// Live smoke test — hits Open-Meteo. `#[ignore]` keeps CI offline-safe. Run with
    /// `cargo test -- --ignored fetcher::weather::forecast::tests::live` to verify the API
    /// shape and weekday rendering for "today + 2".
    #[tokio::test]
    #[ignore]
    async fn live_tokyo_forecast_populates_three_days() {
        let forecast = fetch_forecast(35.68, 139.76, Units::Metric, 3)
            .await
            .unwrap();
        assert_eq!(forecast.days.len(), 3);
        let Body::TextBlock(block) = text_block(&forecast) else {
            panic!("text block");
        };
        for line in &block.lines {
            eprintln!("{line}");
        }
    }
}
