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
        cpu_gauge(),
        memory_sparkline(),
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

fn cpu_gauge() -> Payload {
    titled(
        "CPU",
        Body::Gauge(GaugeData {
            value: 0.63,
            label: Some("63%".into()),
        }),
    )
}

fn memory_sparkline() -> Payload {
    titled(
        "Memory",
        Body::Sparkline(SparklineData {
            values: vec![30, 45, 50, 55, 60, 58, 65, 70, 68, 62, 55, 50],
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
        "PRs",
        Body::BarChart(BarChartData {
            bars: vec![
                Bar {
                    label: "me".into(),
                    value: 3,
                },
                Bar {
                    label: "team".into(),
                    value: 5,
                },
                Bar {
                    label: "bot".into(),
                    value: 2,
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
