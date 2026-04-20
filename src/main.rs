use std::io::{self, stdout};

use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, widgets::Paragraph};

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(1),
        },
    )?;
    terminal.draw(|frame| {
        frame.render_widget(Paragraph::new("Hello splashboard"), frame.area());
    })?;
    println!();
    Ok(())
}
