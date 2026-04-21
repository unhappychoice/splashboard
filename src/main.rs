use std::io::{self, stdout};

use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};

use crate::payload::{Body, Payload, TextData};

mod payload;
mod render;

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(3),
        },
    )?;
    let payload = demo_payload();
    terminal.draw(|frame| render::render_payload(frame, frame.area(), &payload))?;
    println!();
    Ok(())
}

fn demo_payload() -> Payload {
    Payload {
        title: Some("splashboard".into()),
        icon: None,
        status: None,
        format: None,
        body: Body::Text(TextData {
            lines: vec!["Hello splashboard".into()],
        }),
    }
}
