use std::io::{self, IsTerminal, stdout};

use clap::{Parser, Subcommand};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};

use crate::config::Config;
use crate::shell::Shell;

mod config;
mod layout;
mod payload;
mod render;
mod shell;
mod stubs;

#[derive(Parser)]
#[command(version, about = "A customizable terminal splash screen")]
struct Cli {
    /// Render only if the current directory directly holds a config file; otherwise exit
    /// silently. Intended for cd-hook invocations so the splash shows exactly once per project
    /// entry instead of on every subdirectory navigation.
    #[arg(long)]
    on_cd: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Emit a shell init snippet; source it from your rc file to render on new shells and on cd.
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Init { shell }) => {
            print!("{}", shell::init_snippet(shell));
            Ok(())
        }
        None if cli.on_cd => render_for_cd(),
        None => render_splash(),
    }
}

fn render_splash() -> io::Result<()> {
    if !stdout().is_terminal() {
        return Ok(());
    }
    draw(&load_full_config())
}

fn render_for_cd() -> io::Result<()> {
    if !stdout().is_terminal() {
        return Ok(());
    }
    let Some(path) = config::resolve_cwd_only_path() else {
        return Ok(());
    };
    let Ok(config) = Config::load_or_default(&path) else {
        return Ok(());
    };
    draw(&config)
}

fn draw(config: &Config) -> io::Result<()> {
    let root = config.to_layout();
    let widgets = stubs::widgets_for(config.widgets.iter().map(|w| &w.id));
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(16),
        },
    )?;
    terminal.draw(|frame| layout::draw(frame, frame.area(), &root, &widgets))?;
    println!();
    Ok(())
}

fn load_full_config() -> Config {
    config::resolve_config_path()
        .and_then(|p| Config::load_or_default(&p).ok())
        .unwrap_or_else(Config::default_baked)
}
