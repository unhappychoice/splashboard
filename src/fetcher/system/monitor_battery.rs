//! `system_monitor_battery` — primary (or selected) battery state.
//!
//! Ratio pairs with `gauge_battery`; Text is a formatted summary (kind picks the field);
//! Entries rolls up charge / state / time / cycles / health; Badge maps to a status pill.
//! Hosts without a battery (desktops, servers) render a "full AC" stand-in so the widget
//! doesn't disappear.

use std::sync::Mutex;

use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{BadgeData, Body, EntriesData, Entry, Payload, RatioData, Status, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{entry, format_uptime, options_placeholder, parse_options, payload};

const OPTION_SCHEMAS: &[OptionSchema] = &[
    OptionSchema {
        name: "kind",
        type_hint: "\"summary\" | \"percent\" | \"status\" | \"time_remaining\"",
        required: false,
        default: Some("\"summary\""),
        description: "Selects the format of the `Text` shape. Ignored by `Ratio` / `Entries`.",
    },
    OptionSchema {
        name: "index",
        type_hint: "integer",
        required: false,
        default: Some("0"),
        description: "Index of the battery to read on multi-battery systems.",
    },
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatteryOptions {
    #[serde(default)]
    pub kind: Option<BatteryTextKind>,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryTextKind {
    #[default]
    Summary,
    Percent,
    Status,
    TimeRemaining,
}

#[derive(Clone, Copy)]
pub(crate) enum BatteryState {
    Charging,
    Discharging,
    Full,
    Empty,
    Unknown,
}

impl BatteryState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Full => "Full",
            Self::Empty => "Empty",
            Self::Unknown => "Unknown",
        }
    }
}

pub(crate) struct BatterySnapshot {
    pub(crate) charge: f64,
    pub(crate) state: BatteryState,
    pub(crate) time_remaining_secs: Option<u64>,
    pub(crate) cycle_count: Option<u32>,
    pub(crate) health: Option<f64>,
}

pub struct SystemMonitorBattery {
    pub(crate) manager: Mutex<Option<starship_battery::Manager>>,
}

impl SystemMonitorBattery {
    pub fn new() -> Self {
        Self {
            manager: Mutex::new(starship_battery::Manager::new().ok()),
        }
    }
}

impl Default for SystemMonitorBattery {
    fn default() -> Self {
        Self::new()
    }
}

impl RealtimeFetcher for SystemMonitorBattery {
    fn name(&self) -> &str {
        "system_monitor_battery"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Charge level and state of the primary (or `index`-selected) battery. `Ratio` drives gauges, `Text` formats a summary line whose field is picked by `kind`, and `Entries` rolls up charge / state / time-left / cycles / health. Hosts without a battery render a steady `\"AC\"` placeholder."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Ratio, Shape::Text, Shape::Entries, Shape::Badge]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        Some(match shape {
            Shape::Ratio => samples::ratio(0.87, "battery"),
            Shape::Text => samples::text("87% • Charging • 1h 23m"),
            Shape::Entries => samples::entries(&[
                ("charge", "87%"),
                ("state", "Charging"),
                ("time_left", "1h 23m"),
                ("cycles", "284"),
                ("health", "97%"),
            ]),
            Shape::Badge => samples::badge(Status::Ok, "87% · Charging"),
            _ => return None,
        })
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: BatteryOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let snapshot = self.read_snapshot(opts.index.unwrap_or(0));
        let shape = ctx.shape.unwrap_or(Shape::Ratio);
        match (snapshot, shape) {
            (Some(snap), Shape::Text) => payload(Body::Text(TextData {
                value: format_battery_text(&snap, opts.kind.unwrap_or_default()),
            })),
            (Some(snap), Shape::Entries) => payload(Body::Entries(EntriesData {
                items: battery_entries(&snap),
            })),
            (Some(snap), Shape::Badge) => payload(Body::Badge(battery_badge(&snap))),
            (Some(snap), _) => payload(Body::Ratio(RatioData {
                value: snap.charge,
                label: Some(format!(
                    "{} • {}",
                    format_percent(snap.charge),
                    snap.state.label()
                )),
                denominator: None,
            })),
            (None, shape) => no_battery_payload(shape, opts.kind.unwrap_or_default()),
        }
    }
}

impl SystemMonitorBattery {
    fn read_snapshot(&self, index: usize) -> Option<BatterySnapshot> {
        let manager = self.manager.lock().unwrap_or_else(|e| e.into_inner());
        let manager = manager.as_ref()?;
        let battery = manager.batteries().ok()?.nth(index)?.ok()?;
        Some(snapshot_from(&battery))
    }
}

fn snapshot_from(battery: &starship_battery::Battery) -> BatterySnapshot {
    BatterySnapshot {
        charge: f64::from(battery.state_of_charge().value).clamp(0.0, 1.0),
        state: map_battery_state(battery.state()),
        time_remaining_secs: time_remaining_secs(battery),
        cycle_count: battery.cycle_count(),
        health: battery_health(battery),
    }
}

pub(crate) fn map_battery_state(s: starship_battery::State) -> BatteryState {
    use starship_battery::State as S;
    match s {
        S::Charging => BatteryState::Charging,
        S::Discharging => BatteryState::Discharging,
        S::Full => BatteryState::Full,
        S::Empty => BatteryState::Empty,
        _ => BatteryState::Unknown,
    }
}

fn time_remaining_secs(battery: &starship_battery::Battery) -> Option<u64> {
    let dur = match battery.state() {
        starship_battery::State::Charging => battery.time_to_full(),
        starship_battery::State::Discharging => battery.time_to_empty(),
        _ => None,
    }?;
    Some(dur.value.max(0.0) as u64)
}

fn battery_health(battery: &starship_battery::Battery) -> Option<f64> {
    let full = f64::from(battery.energy_full().value);
    let design = f64::from(battery.energy_full_design().value);
    if design <= 0.0 {
        None
    } else {
        Some((full / design).clamp(0.0, 1.0))
    }
}

pub(crate) fn format_battery_text(snap: &BatterySnapshot, kind: BatteryTextKind) -> String {
    match kind {
        BatteryTextKind::Percent => format_percent(snap.charge),
        BatteryTextKind::Status => snap.state.label().into(),
        BatteryTextKind::TimeRemaining => snap
            .time_remaining_secs
            .map(format_uptime)
            .unwrap_or_else(|| "—".into()),
        BatteryTextKind::Summary => match snap.time_remaining_secs {
            Some(secs) => format!(
                "{} • {} • {}",
                format_percent(snap.charge),
                snap.state.label(),
                format_uptime(secs)
            ),
            None => format!("{} • {}", format_percent(snap.charge), snap.state.label()),
        },
    }
}

pub(crate) fn battery_badge(snap: &BatterySnapshot) -> BadgeData {
    let status = match snap.state {
        BatteryState::Charging | BatteryState::Full => Status::Ok,
        _ if snap.charge < 0.20 => Status::Error,
        _ if snap.charge < 0.50 => Status::Warn,
        _ => Status::Ok,
    };
    BadgeData {
        status,
        label: format!("{} · {}", format_percent(snap.charge), snap.state.label()),
    }
}

pub(crate) fn battery_entries(snap: &BatterySnapshot) -> Vec<Entry> {
    let mut items = vec![
        entry("charge", &format_percent(snap.charge)),
        entry("state", snap.state.label()),
    ];
    if let Some(secs) = snap.time_remaining_secs {
        items.push(entry("time_left", &format_uptime(secs)));
    }
    if let Some(cycles) = snap.cycle_count {
        items.push(entry("cycles", &cycles.to_string()));
    }
    if let Some(h) = snap.health {
        items.push(entry("health", &format_percent(h)));
    }
    items
}

pub(crate) fn no_battery_payload(shape: Shape, kind: BatteryTextKind) -> Payload {
    match shape {
        Shape::Text => payload(Body::Text(TextData {
            value: match kind {
                BatteryTextKind::Percent => "100%".into(),
                BatteryTextKind::TimeRemaining => "—".into(),
                _ => "AC".into(),
            },
        })),
        Shape::Entries => payload(Body::Entries(EntriesData {
            items: vec![entry("power", "AC")],
        })),
        Shape::Badge => payload(Body::Badge(BadgeData {
            status: Status::Ok,
            label: "AC".into(),
        })),
        _ => payload(Body::Ratio(RatioData {
            value: 1.0,
            label: Some("AC".into()),
            denominator: None,
        })),
    }
}

pub(crate) fn format_percent(ratio: f64) -> String {
    format!("{:.0}%", ratio.clamp(0.0, 1.0) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::{ctx_text, ctx_with_shape};
    use super::*;

    fn snapshot(charge: f64, state: BatteryState, secs: Option<u64>) -> BatterySnapshot {
        BatterySnapshot {
            charge,
            state,
            time_remaining_secs: secs,
            cycle_count: Some(284),
            health: Some(0.97),
        }
    }

    #[test]
    fn sample_body_matches_documented_shapes() {
        let fetcher = SystemMonitorBattery::default();

        assert!(matches!(
            fetcher.sample_body(Shape::Ratio),
            Some(Body::Ratio(ratio))
                if ratio.label.as_deref() == Some("battery") && ratio.value == 0.87
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Text),
            Some(Body::Text(text)) if text.value == "87% • Charging • 1h 23m"
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Entries),
            Some(Body::Entries(entries))
                if entries.items.len() == 5
                    && entries.items[0].key == "charge"
                    && entries.items[4].value.as_deref() == Some("97%")
        ));
        assert!(matches!(
            fetcher.sample_body(Shape::Badge),
            Some(Body::Badge(badge))
                if badge.status == Status::Ok && badge.label == "87% · Charging"
        ));
    }

    #[test]
    fn summary_includes_time_when_available() {
        let snap = snapshot(0.87, BatteryState::Charging, Some(83 * 60));
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::Summary),
            "87% • Charging • 1h 23m"
        );
    }

    #[test]
    fn summary_omits_time_when_missing() {
        let snap = snapshot(1.0, BatteryState::Full, None);
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::Summary),
            "100% • Full"
        );
    }

    #[test]
    fn text_kinds_pick_distinct_fields() {
        let snap = snapshot(0.5, BatteryState::Discharging, Some(45 * 60));
        assert_eq!(format_battery_text(&snap, BatteryTextKind::Percent), "50%");
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::Status),
            "Discharging"
        );
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::TimeRemaining),
            "45m"
        );
    }

    #[test]
    fn time_remaining_dash_when_missing() {
        let snap = snapshot(1.0, BatteryState::Full, None);
        assert_eq!(
            format_battery_text(&snap, BatteryTextKind::TimeRemaining),
            "—"
        );
    }

    #[test]
    fn state_mapping_and_labels_cover_all_variants() {
        use starship_battery::State;

        assert_eq!(map_battery_state(State::Charging).label(), "Charging");
        assert_eq!(map_battery_state(State::Discharging).label(), "Discharging");
        assert_eq!(map_battery_state(State::Full).label(), "Full");
        assert_eq!(map_battery_state(State::Empty).label(), "Empty");
        assert_eq!(map_battery_state(State::Unknown).label(), "Unknown");
    }

    #[test]
    fn entries_include_optional_fields_only_when_present() {
        let with = snapshot(0.5, BatteryState::Charging, Some(60));
        let mut without = snapshot(0.5, BatteryState::Charging, None);
        without.cycle_count = None;
        without.health = None;
        assert_eq!(battery_entries(&with).len(), 5);
        assert_eq!(battery_entries(&without).len(), 2);
    }

    #[test]
    fn no_battery_ratio_is_full_ac() {
        let p = no_battery_payload(Shape::Ratio, BatteryTextKind::Summary);
        assert!(matches!(
            p.body,
            Body::Ratio(r) if r.value == 1.0 && r.label.as_deref() == Some("AC"),
        ));
    }

    #[test]
    fn no_battery_text_varies_by_kind() {
        let summary = no_battery_payload(Shape::Text, BatteryTextKind::Summary);
        let percent = no_battery_payload(Shape::Text, BatteryTextKind::Percent);
        let time = no_battery_payload(Shape::Text, BatteryTextKind::TimeRemaining);
        assert!(matches!(summary.body, Body::Text(t) if t.value == "AC"));
        assert!(matches!(percent.body, Body::Text(t) if t.value == "100%"));
        assert!(matches!(time.body, Body::Text(t) if t.value == "—"));
    }

    #[test]
    fn no_battery_entries_and_badge_use_ac_placeholders() {
        let entries = no_battery_payload(Shape::Entries, BatteryTextKind::Summary);
        let badge = no_battery_payload(Shape::Badge, BatteryTextKind::Summary);
        assert!(matches!(
            entries.body,
            Body::Entries(d)
                if d.items.len() == 1
                    && d.items[0].key == "power"
                    && d.items[0].value.as_deref() == Some("AC"),
        ));
        assert!(matches!(
            badge.body,
            Body::Badge(d) if d.status == Status::Ok && d.label == "AC",
        ));
    }

    #[test]
    fn compute_never_panics_on_any_shape() {
        let f = SystemMonitorBattery::new();
        for shape in [
            None,
            Some(Shape::Ratio),
            Some(Shape::Text),
            Some(Shape::Entries),
            Some(Shape::Badge),
        ] {
            let p = f.compute(&ctx_with_shape(shape));
            assert!(!matches!(p.body, Body::Image(_)));
        }
    }

    #[test]
    fn badge_status_reflects_charge_and_state() {
        let low = snapshot(0.05, BatteryState::Discharging, None);
        assert_eq!(battery_badge(&low).status, Status::Error);
        let mid = snapshot(0.30, BatteryState::Discharging, None);
        assert_eq!(battery_badge(&mid).status, Status::Warn);
        let high = snapshot(0.95, BatteryState::Discharging, None);
        assert_eq!(battery_badge(&high).status, Status::Ok);
        let charging_low = snapshot(0.05, BatteryState::Charging, None);
        assert_eq!(battery_badge(&charging_low).status, Status::Ok);
    }

    #[test]
    fn rejects_unknown_option_to_placeholder() {
        let f = SystemMonitorBattery::new();
        let p = f.compute(&ctx_text(Some("bogus = true")));
        assert!(matches!(p.body, Body::Text(t) if t.value.starts_with('⚠')));
    }

    #[test]
    fn compute_without_manager_uses_no_battery_fallbacks() {
        let fetcher = SystemMonitorBattery {
            manager: Mutex::new(None),
        };

        let text = fetcher.compute(&ctx_text(Some("kind = \"percent\"")));
        let entries = fetcher.compute(&ctx_with_shape(Some(Shape::Entries)));
        let badge = fetcher.compute(&ctx_with_shape(Some(Shape::Badge)));
        assert!(matches!(text.body, Body::Text(t) if t.value == "100%"));
        assert!(matches!(
            entries.body,
            Body::Entries(d) if d.items[0].value.as_deref() == Some("AC"),
        ));
        assert!(matches!(badge.body, Body::Badge(b) if b.label == "AC"));
    }
}
