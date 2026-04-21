use std::collections::HashMap;
use std::io::{self, stdout};

use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};

use crate::layout::{Child, Layout, WidgetId};
use crate::payload::{
    Bar, BarChartData, BignumData, Body, GaugeData, ListData, ListItem, Payload, SparklineData,
    Status, TextData,
};

mod layout;
mod payload;
mod render;

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(16),
        },
    )?;
    let root = demo_layout();
    let widgets = demo_widgets();
    terminal.draw(|frame| layout::draw(frame, frame.area(), &root, &widgets))?;
    println!();
    Ok(())
}

fn demo_layout() -> Layout {
    Layout::rows(vec![
        Child::min(6, row(&["greeting", "clock"])),
        Child::length(5, row(&["disk", "commits"])),
        Child::length(5, row(&["system", "prs"])),
    ])
}

fn row(ids: &[&str]) -> Layout {
    Layout::cols(
        ids.iter()
            .map(|id| Child::fill(1, Layout::widget(*id)))
            .collect(),
    )
}

fn demo_widgets() -> HashMap<WidgetId, Payload> {
    [
        ("greeting", greeting()),
        ("clock", clock()),
        ("disk", disk_gauge()),
        ("commits", commits_sparkline()),
        ("system", system_list()),
        ("prs", pr_counts()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

fn greeting() -> Payload {
    titled(
        "Greeting",
        Body::Text(TextData {
            lines: vec!["Hello, splashboard!".into()],
        }),
    )
}

fn clock() -> Payload {
    titled(
        "Clock",
        Body::Bignum(BignumData {
            text: "12:34".into(),
        }),
    )
}

fn disk_gauge() -> Payload {
    titled(
        "Disk /",
        Body::Gauge(GaugeData {
            value: 0.45,
            label: Some("45% of 500 GB".into()),
        }),
    )
}

fn commits_sparkline() -> Payload {
    titled(
        "Commits (14d)",
        Body::Sparkline(SparklineData {
            values: vec![2, 5, 0, 3, 7, 4, 1, 6, 9, 2, 3, 5, 8, 4],
        }),
    )
}

fn system_list() -> Payload {
    let ok = Some(Status::Ok);
    titled(
        "System",
        Body::List(ListData {
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
        }),
    )
}

fn pr_counts() -> Payload {
    titled(
        "Open PRs",
        Body::BarChart(BarChartData {
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
        }),
    )
}

fn titled(title: &str, body: Body) -> Payload {
    Payload {
        title: Some(title.into()),
        icon: None,
        status: None,
        format: None,
        body,
    }
}
