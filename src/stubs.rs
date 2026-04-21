use std::collections::HashMap;

use crate::layout::WidgetId;
use crate::payload::{
    Bar, BarChartData, BignumData, Body, GaugeData, ListData, ListItem, Payload, SparklineData,
    Status, TextData,
};

pub fn widgets_for(ids: impl IntoIterator<Item = impl AsRef<str>>) -> HashMap<WidgetId, Payload> {
    ids.into_iter()
        .filter_map(|id| {
            let id = id.as_ref();
            stub_payload(id).map(|p| (id.to_string(), p))
        })
        .collect()
}

fn stub_payload(id: &str) -> Option<Payload> {
    match id {
        "greeting" => Some(greeting()),
        "clock" => Some(clock()),
        "disk" => Some(disk_gauge()),
        "commits" => Some(commits_sparkline()),
        "system" => Some(system_list()),
        "prs" => Some(pr_counts()),
        _ => None,
    }
}

fn greeting() -> Payload {
    payload(Body::Text(TextData {
        lines: vec!["Hello, splashboard!".into()],
    }))
}

fn clock() -> Payload {
    payload(Body::Bignum(BignumData {
        text: "12:34".into(),
    }))
}

fn disk_gauge() -> Payload {
    payload(Body::Gauge(GaugeData {
        value: 0.45,
        label: Some("45% of 500 GB".into()),
    }))
}

fn commits_sparkline() -> Payload {
    payload(Body::Sparkline(SparklineData {
        values: vec![2, 5, 0, 3, 7, 4, 1, 6, 9, 2, 3, 5, 8, 4],
    }))
}

fn system_list() -> Payload {
    let ok = Some(Status::Ok);
    payload(Body::List(ListData {
        items: vec![
            ListItem {
                key: "os".into(),
                value: Some("linux".into()),
                status: ok,
            },
            ListItem {
                key: "uptime".into(),
                value: Some("3d 2h".into()),
                status: ok,
            },
            ListItem {
                key: "load".into(),
                value: Some("0.28".into()),
                status: ok,
            },
        ],
    }))
}

fn pr_counts() -> Payload {
    payload(Body::BarChart(BarChartData {
        bars: vec![
            Bar {
                label: "splsh".into(),
                value: 3,
            },
            Bar {
                label: "gtype".into(),
                value: 2,
            },
            Bar {
                label: "other".into(),
                value: 1,
            },
        ],
    }))
}

fn payload(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}
