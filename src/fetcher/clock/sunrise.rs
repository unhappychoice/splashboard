//! `clock_sunrise` — sunrise & sunset for a lat/lon, via a NOAA-style approximation (±1 min).
//! Emits `Text` (`"↑ 05:23  ↓ 18:47"`) or `Entries` (split rows).

use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, TimeZone, Timelike, Utc};
use serde::Deserialize;

use crate::fetcher::{FetchContext, RealtimeFetcher, Safety};
use crate::options::OptionSchema;
use crate::payload::{Body, EntriesData, Entry, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::common;

const SHAPES: &[Shape] = &[Shape::Text, Shape::Entries];

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "lat",
        type_hint: "number (degrees, -90..=90)",
        required: true,
        default: None,
        description: "Observer latitude. Required — without it sunrise/sunset can't be computed.",
    },
    OptionSchema {
        name: "lon",
        type_hint: "number (degrees, -180..=180)",
        required: true,
        default: None,
        description: "Observer longitude. Required.",
    },
    OptionSchema {
        name: "timezone",
        type_hint: "IANA timezone (e.g. \"Asia/Tokyo\")",
        required: false,
        default: Some("system local"),
        description: "Timezone the rendered clock times are displayed in. Calculation is done in UTC regardless.",
    },
];

pub struct ClockSunriseFetcher;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
}

impl RealtimeFetcher for ClockSunriseFetcher {
    fn name(&self) -> &str {
        "clock_sunrise"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Text => samples::text("↑ 05:23  ↓ 18:47"),
            Shape::Entries => samples::entries(&[("sunrise", "05:23"), ("sunset", "18:47")]),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: Options = match common::parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return common::placeholder(&msg),
        };
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let body = match compute_events(&opts) {
            Ok((up, down)) => match shape {
                Shape::Entries => Body::Entries(EntriesData {
                    items: vec![
                        entry("sunrise", &format_clock(&up)),
                        entry("sunset", &format_clock(&down)),
                    ],
                }),
                _ => Body::Text(TextData {
                    value: format!("↑ {}  ↓ {}", format_clock(&up), format_clock(&down)),
                }),
            },
            Err(msg) => Body::Text(TextData { value: msg }),
        };
        Payload {
            icon: None,
            status: None,
            format: None,
            body,
        }
    }
}

fn compute_events(
    opts: &Options,
) -> Result<(DateTime<FixedOffset>, DateTime<FixedOffset>), String> {
    let lat = opts.lat.ok_or("lat required")?;
    let lon = opts.lon.ok_or("lon required")?;
    let now = common::now_in(opts.timezone.as_deref());
    let date = now.date_naive();
    let offset = *now.offset();
    let (up, down) =
        solar_events(date, lat, lon).ok_or("no sunrise/sunset at this latitude today")?;
    Ok((to_local(date, up, offset), to_local(date, down, offset)))
}

fn solar_events(date: NaiveDate, lat: f64, lon: f64) -> Option<(f64, f64)> {
    let n = (date - NaiveDate::from_ymd_opt(date.year(), 1, 1)?).num_days() as f64 + 1.0;
    let lng_hour = lon / 15.0;
    Some((
        event(n, lat, lng_hour, true)?,
        event(n, lat, lng_hour, false)?,
    ))
}

/// NOAA sunrise equation. Returns the event time as UTC hours-of-day, or `None` for polar
/// day/night (where `acos` is undefined).
fn event(n: f64, lat: f64, lng_hour: f64, rising: bool) -> Option<f64> {
    let t = if rising {
        n + (6.0 - lng_hour) / 24.0
    } else {
        n + (18.0 - lng_hour) / 24.0
    };
    let m = (0.985_600_28 * t) - 3.289;
    let l = m + 1.916 * m.to_radians().sin() + 0.020 * (2.0 * m).to_radians().sin() + 282.634;
    let l = l.rem_euclid(360.0);
    let mut ra = (0.917_85 * l.to_radians().tan()).atan().to_degrees();
    ra = ra.rem_euclid(360.0);
    let l_q = (l / 90.0).floor() * 90.0;
    let ra_q = (ra / 90.0).floor() * 90.0;
    ra = (ra + (l_q - ra_q)) / 15.0;
    let sin_dec = 0.397_8 * l.to_radians().sin();
    let cos_dec = (1.0 - sin_dec * sin_dec).sqrt();
    let zenith = 90.833_f64.to_radians();
    let cos_h =
        (zenith.cos() - sin_dec * lat.to_radians().sin()) / (cos_dec * lat.to_radians().cos());
    if !(-1.0..=1.0).contains(&cos_h) {
        return None;
    }
    let h = if rising {
        360.0 - cos_h.acos().to_degrees()
    } else {
        cos_h.acos().to_degrees()
    } / 15.0;
    let t_utc = h + ra - (0.065_71 * t) - 6.622;
    Some((t_utc - lng_hour).rem_euclid(24.0))
}

fn to_local(date: NaiveDate, utc_hours: f64, offset: FixedOffset) -> DateTime<FixedOffset> {
    let whole_h = utc_hours.floor() as u32;
    let minutes = ((utc_hours - utc_hours.floor()) * 60.0).round() as u32;
    let (h, m) = if minutes >= 60 {
        ((whole_h + 1) % 24, 0)
    } else {
        (whole_h % 24, minutes)
    };
    let naive = date
        .and_hms_opt(h, m, 0)
        .unwrap_or_else(|| date.and_hms_opt(0, 0, 0).expect("midnight always valid"));
    Utc.from_utc_datetime(&naive).with_timezone(&offset)
}

fn entry(key: &str, value: &str) -> Entry {
    Entry {
        key: key.into(),
        value: Some(value.into()),
        status: None,
    }
}

fn format_clock(dt: &DateTime<FixedOffset>) -> String {
    format!("{:02}:{:02}", dt.hour(), dt.minute())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn ctx(shape: Option<Shape>, options: &str) -> FetchContext {
        FetchContext {
            widget_id: "sun".into(),
            timeout: Duration::from_secs(1),
            shape,
            options: Some(toml::from_str(options).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn missing_latlon_renders_error_text() {
        let p = ClockSunriseFetcher.compute(&ctx(Some(Shape::Text), ""));
        match p.body {
            Body::Text(d) => assert!(d.value.contains("required")),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn tokyo_returns_plausible_times() {
        let p = ClockSunriseFetcher.compute(&ctx(
            Some(Shape::Text),
            r#"lat = 35.6762
lon = 139.6503
timezone = "Asia/Tokyo""#,
        ));
        match p.body {
            Body::Text(d) => {
                assert!(d.value.contains("↑"));
                assert!(d.value.contains("↓"));
            }
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn entries_shape_emits_sunrise_and_sunset_rows() {
        let p = ClockSunriseFetcher.compute(&ctx(
            Some(Shape::Entries),
            r#"lat = 35.6762
lon = 139.6503"#,
        ));
        match p.body {
            Body::Entries(d) => {
                let keys: Vec<_> = d.items.iter().map(|e| e.key.as_str()).collect();
                assert_eq!(keys, ["sunrise", "sunset"]);
            }
            _ => panic!("expected entries"),
        }
    }
}
