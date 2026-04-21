use std::io::{self, stdout};

use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
};

use crate::payload::{
    Bar, BarChartData, BignumData, Body, GaugeData, ListData, ListItem, Payload, SparklineData,
    Status, TextData,
};

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
    terminal.draw(draw_demo)?;
    println!();
    Ok(())
}

fn draw_demo(frame: &mut Frame) {
    let widgets = demo_widgets();
    let rows = Layout::vertical([
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(5),
    ])
    .split(frame.area());
    for (row_idx, row) in rows.iter().enumerate() {
        let cols = Layout::horizontal([Constraint::Percentage(50); 2]).split(*row);
        render::render_payload(frame, cols[0], &widgets[row_idx * 2]);
        render::render_payload(frame, cols[1], &widgets[row_idx * 2 + 1]);
    }
}

fn demo_widgets() -> [Payload; 6] {
    [
        greeting(),
        clock(),
        disk_gauge(),
        commits_sparkline(),
        system_list(),
        pr_counts(),
    ]
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
