//! Fullscreen gallery picker used when the user runs `splashboard install` in a TTY
//! without supplying the matching flags. Four screens, each optional depending on what
//! the caller still needs: home template, project template, theme preset, and a toggle
//! screen for bg / wait. Screens share a single alternate-screen session so the user
//! sees a continuous flow rather than multiple open / close flickers.

use std::io::{self, Stdout, stdout};
use std::path::PathBuf;
use std::time::Duration;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout as UiLayout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::templates::{Template, TemplateContext, for_context};
use crate::theme::Theme;
use crate::theme::presets as theme_presets;

use super::SettingsChoices;
use super::preview::TemplatePreview;

/// What the caller still needs the picker to fill in. Any combination may be off — if
/// the user pre-supplied templates and a theme via flags but skipped `--no-bg`, only
/// the toggle screen runs.
pub(crate) struct Needs {
    pub(crate) templates: bool,
    pub(crate) settings: bool,
}

/// Fully-resolved picker output. Each field is `None` when the corresponding screen was
/// skipped (i.e. the caller already has that value).
pub(crate) struct Selection {
    pub(crate) templates: Option<TemplatePair>,
    pub(crate) settings: Option<SettingsChoices>,
}

pub(crate) struct TemplatePair {
    pub(crate) home: &'static Template,
    pub(crate) project: &'static Template,
}

/// Runs whatever subset of screens `needs` requests, returning `None` if the user
/// cancels at any point so the caller can bail without touching any files.
pub(crate) fn run(
    needs: Needs,
    home_for_theme_preview: Option<&'static Template>,
) -> io::Result<Option<Selection>> {
    let mut terminal = enter_terminal()?;
    let result = run_screens(&mut terminal, needs, home_for_theme_preview);
    exit_terminal(terminal)?;
    result
}

fn run_screens(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    needs: Needs,
    home_for_theme_preview: Option<&'static Template>,
) -> io::Result<Option<Selection>> {
    let templates = if needs.templates {
        let Some(pair) = run_template_picker_pair(terminal)? else {
            return Ok(None);
        };
        Some(pair)
    } else {
        None
    };

    let settings = if needs.settings {
        // For the theme picker preview we want the home template the user just picked (or
        // the one they supplied via flag). Fall back to the first home template in the
        // catalog when neither is available so the preview still shows something.
        let preview_template = templates
            .as_ref()
            .map(|p| p.home)
            .or(home_for_theme_preview)
            .or_else(|| for_context(TemplateContext::Home).next());
        let Some(choices) = run_settings_pickers(terminal, preview_template)? else {
            return Ok(None);
        };
        Some(choices)
    } else {
        None
    };

    Ok(Some(Selection {
        templates,
        settings,
    }))
}

fn enter_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    Terminal::new(backend)
}

fn exit_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

// ── Confirmation ──────────────────────────────────────────────────────────────────

/// Snapshot of the resolved install plan shown on the final confirmation page. Built by
/// `install::run` after merging flag defaults with picker output so every line reflects
/// what would actually land on disk, not the intermediate picker state.
pub(crate) struct Summary {
    pub(crate) shell: String,
    pub(crate) home_template: String,
    pub(crate) project_template: String,
    pub(crate) theme: String,
    pub(crate) bg: bool,
    pub(crate) wait: bool,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) rc_path: Option<PathBuf>,
}

/// Opens a new alt-screen session and shows the final confirmation page. Returns `true`
/// when the user confirms (Enter), `false` on cancel (Esc / q / Ctrl-C). The caller
/// should only show this when pickers actually ran — scripted invocations skip it.
pub(crate) fn confirm(summary: Summary) -> io::Result<bool> {
    let mut terminal = enter_terminal()?;
    let result = confirm_loop(&mut terminal, &summary);
    exit_terminal(terminal)?;
    result
}

fn confirm_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    summary: &Summary,
) -> io::Result<bool> {
    loop {
        terminal.draw(|frame| draw_confirmation(frame, summary))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) | (KeyCode::Char('y'), _) | (KeyCode::Char('Y'), _) => {
                return Ok(true);
            }
            (KeyCode::Esc, _)
            | (KeyCode::Char('q'), _)
            | (KeyCode::Char('n'), _)
            | (KeyCode::Char('N'), _)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(false),
            _ => {}
        }
    }
}

fn draw_confirmation(frame: &mut Frame, summary: &Summary) {
    let meta = SlotMeta {
        title: "Review and confirm",
        subtitle: "Existing files that differ get moved to a `.bak` sidecar before the new content lands.",
    };
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(8),    // body
            Constraint::Length(1), // footer
        ])
        .split(area);
    draw_header(frame, layout[0], &meta, 1, 1);

    let block = Block::default().borders(Borders::ALL).title(" summary ");
    let inner = block.inner(layout[1]);
    frame.render_widget(block, layout[1]);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(summary_row("Shell", &summary.shell));
    lines.push(summary_row("Home dashboard", &summary.home_template));
    lines.push(summary_row("Project dashboard", &summary.project_template));
    lines.push(summary_row("Theme", &summary.theme));
    lines.push(summary_row(
        "Paint splash bg",
        if summary.bg {
            "on"
        } else {
            "off (terminal bg shows through)"
        },
    ));
    lines.push(summary_row(
        "Wait for fresh data",
        if summary.wait { "on" } else { "off" },
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Files to write:",
        Style::default().fg(Color::White),
    )));
    for path in &summary.files {
        lines.push(Line::from(Span::styled(
            format!("  · {}", path.display()),
            Style::default().fg(Color::DarkGray),
        )));
    }
    if let Some(rc) = &summary.rc_path {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Shell rc block:",
            Style::default().fg(Color::White),
        )));
        lines.push(Line::from(Span::styled(
            format!("  · {}", rc.display()),
            Style::default().fg(Color::DarkGray),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
    draw_footer(frame, layout[2], "Enter / y: write · Esc / q / n: cancel");
}

fn summary_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<22}"), Style::default().fg(Color::White)),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

// ── Template pickers ──────────────────────────────────────────────────────────────

fn run_template_picker_pair(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<Option<TemplatePair>> {
    let home_templates: Vec<&'static Template> = for_context(TemplateContext::Home).collect();
    let project_templates: Vec<&'static Template> = for_context(TemplateContext::Project).collect();

    if home_templates.is_empty() || project_templates.is_empty() {
        return Ok(None);
    }

    let Some(home) = run_template_picker(terminal, &home_templates, SlotMeta::home())? else {
        return Ok(None);
    };
    let Some(project) = run_template_picker(terminal, &project_templates, SlotMeta::project())?
    else {
        return Ok(None);
    };
    Ok(Some(TemplatePair { home, project }))
}

struct SlotMeta {
    title: &'static str,
    subtitle: &'static str,
}

impl SlotMeta {
    fn home() -> Self {
        Self {
            title: "Pick your home dashboard",
            subtitle: "Shown on new shells and when you cd outside any git project.",
        }
    }

    fn project() -> Self {
        Self {
            title: "Pick your default project dashboard",
            subtitle: "Shown automatically when you cd into the top of any git repo that doesn't ship its own .splashboard/.",
        }
    }
}

fn run_template_picker(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    templates: &[&'static Template],
    meta: SlotMeta,
) -> io::Result<Option<&'static Template>> {
    let previews: Vec<Option<TemplatePreview>> = templates
        .iter()
        .map(|t| TemplatePreview::from_body(t.body))
        .collect();

    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| {
            draw_template_frame(
                frame,
                &meta,
                templates,
                selected,
                previews[selected].as_ref(),
            );
        })?;

        match wait_for_action(templates.len(), selected)? {
            Action::Cancel => return Ok(None),
            Action::Confirm => return Ok(Some(templates[selected])),
            Action::Move(to) => selected = to,
            Action::Toggle | Action::Tick => {}
        }
    }
}

fn draw_template_frame(
    frame: &mut Frame,
    meta: &SlotMeta,
    templates: &[&'static Template],
    selected: usize,
    preview: Option<&TemplatePreview>,
) {
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                          // header
            Constraint::Min(8),                             // preview
            Constraint::Length(templates.len() as u16 + 2), // list
            Constraint::Length(1),                          // footer
        ])
        .split(area);

    draw_header(frame, layout[0], meta, selected + 1, templates.len());
    draw_preview(frame, layout[1], templates[selected].name, preview);
    draw_template_list(frame, layout[2], templates, selected);
    draw_footer(frame, layout[3], "↑/↓ navigate · Enter select · Esc cancel");
}

fn draw_template_list(
    frame: &mut Frame,
    area: Rect,
    templates: &[&'static Template],
    selected: usize,
) {
    let block = Block::default().borders(Borders::ALL).title(" templates ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = templates
        .iter()
        .enumerate()
        .map(|(i, t)| render_name_desc_row(t.name, t.description, i == selected))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Theme picker ──────────────────────────────────────────────────────────────────

fn run_settings_pickers(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    preview_template: Option<&'static Template>,
) -> io::Result<Option<SettingsChoices>> {
    let theme = match run_theme_picker(terminal, preview_template)? {
        ThemePick::Cancelled => return Ok(None),
        ThemePick::Chose(key) => key,
    };
    let Some((bg, wait)) = run_toggle_picker(terminal)? else {
        return Ok(None);
    };
    Ok(Some(SettingsChoices { theme, bg, wait }))
}

/// Outcome of the theme screen. Flat three-way result avoids the
/// `Option<Option<&str>>` double-nesting — `Chose(None)` means "user picked `default`,
/// which writes no preset line", distinct from `Cancelled`.
enum ThemePick {
    Cancelled,
    Chose(Option<&'static str>),
}

/// Candidate rows in the theme picker. `None` maps to the built-in Splash palette (i.e.
/// no `preset = "..."` line in settings.toml) — explicit first slot so the default
/// doesn't feel like "no theme" but "the theme you get by default".
struct ThemeOption {
    key: Option<&'static str>,
    label: &'static str,
    description: &'static str,
}

fn theme_options() -> Vec<ThemeOption> {
    // Hand-written so the order / copy isn't driven by alphabetical preset names —
    // "default" reads better first and the descriptions let users choose by vibe.
    // Light themes go at the bottom so the picker stays dark-first in the dominant case.
    vec![
        ThemeOption {
            key: None,
            label: "default",
            description: "Splash signature — deep-ocean navy with coral titles.",
        },
        ThemeOption {
            key: Some("tokyo_night"),
            label: "tokyo_night",
            description: "Tokyo Night — vibrant blue night, magenta accents.",
        },
        ThemeOption {
            key: Some("tokyo_night_storm"),
            label: "tokyo_night_storm",
            description: "Tokyo Night Storm — softer storm-cloud variant.",
        },
        ThemeOption {
            key: Some("catppuccin_mocha"),
            label: "catppuccin_mocha",
            description: "Catppuccin Mocha — pastel dark, lavender accents.",
        },
        ThemeOption {
            key: Some("catppuccin_macchiato"),
            label: "catppuccin_macchiato",
            description: "Catppuccin Macchiato — slightly lighter than Mocha.",
        },
        ThemeOption {
            key: Some("catppuccin_frappe"),
            label: "catppuccin_frappe",
            description: "Catppuccin Frappé — warmer pastel mid-dark.",
        },
        ThemeOption {
            key: Some("dracula"),
            label: "dracula",
            description: "Dracula — dark purple base, vivid neon accents.",
        },
        ThemeOption {
            key: Some("nord"),
            label: "nord",
            description: "Nord — cool blue-gray arctic palette.",
        },
        ThemeOption {
            key: Some("gruvbox_dark"),
            label: "gruvbox_dark",
            description: "Gruvbox Dark — warm retro, earthy palette.",
        },
        ThemeOption {
            key: Some("rose_pine"),
            label: "rose_pine",
            description: "Rosé Pine — soho dark with rose and pine highlights.",
        },
        ThemeOption {
            key: Some("rose_pine_moon"),
            label: "rose_pine_moon",
            description: "Rosé Pine Moon — moonlit blue-violet variant.",
        },
        ThemeOption {
            key: Some("kanagawa"),
            label: "kanagawa",
            description: "Kanagawa — Hokusai-inspired indigo with autumn accents.",
        },
        ThemeOption {
            key: Some("everforest_dark"),
            label: "everforest_dark",
            description: "Everforest Dark — woodland green with warm muted accents.",
        },
        ThemeOption {
            key: Some("one_dark"),
            label: "one_dark",
            description: "One Dark — Atom's flagship dark, balanced cyan/red.",
        },
        ThemeOption {
            key: Some("solarized_dark"),
            label: "solarized_dark",
            description: "Solarized Dark — the precision-engineered classic.",
        },
        ThemeOption {
            key: Some("monokai"),
            label: "monokai",
            description: "Monokai — punchy magenta and lime on warm gray.",
        },
        ThemeOption {
            key: Some("night_owl"),
            label: "night_owl",
            description: "Night Owl — deep midnight blue, calm accents.",
        },
        ThemeOption {
            key: Some("synthwave_84"),
            label: "synthwave_84",
            description: "SynthWave '84 — neon pink and cyan retro grid.",
        },
        ThemeOption {
            key: Some("ayu_mirage"),
            label: "ayu_mirage",
            description: "Ayu Mirage — soft slate with bright sky and lime.",
        },
        ThemeOption {
            key: Some("material_palenight"),
            label: "material_palenight",
            description: "Material Palenight — material design dark with lavender.",
        },
        ThemeOption {
            key: Some("github_dark"),
            label: "github_dark",
            description: "GitHub Dark — the github.com dark mode palette.",
        },
        ThemeOption {
            key: Some("catppuccin_latte"),
            label: "catppuccin_latte",
            description: "Catppuccin Latte (light) — pastel on cream.",
        },
        ThemeOption {
            key: Some("rose_pine_dawn"),
            label: "rose_pine_dawn",
            description: "Rosé Pine Dawn (light) — warm cream with pine accents.",
        },
        ThemeOption {
            key: Some("solarized_light"),
            label: "solarized_light",
            description: "Solarized Light — the classic light counterpart.",
        },
        ThemeOption {
            key: Some("gruvbox_light"),
            label: "gruvbox_light",
            description: "Gruvbox Light — earthy retro on warm cream.",
        },
        ThemeOption {
            key: Some("github_light"),
            label: "github_light",
            description: "GitHub Light — the github.com light mode palette.",
        },
    ]
}

fn run_theme_picker(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    preview_template: Option<&'static Template>,
) -> io::Result<ThemePick> {
    let options = theme_options();
    // Pre-build a themed preview per option. When no home template is available (the
    // catalog is empty — shouldn't happen in practice) we fall through to `None` and
    // render an "(preview unavailable)" panel instead.
    let previews: Vec<Option<TemplatePreview>> = options
        .iter()
        .map(|opt| {
            preview_template.and_then(|t| {
                let theme = resolve_theme(opt.key);
                TemplatePreview::from_body_with_theme(t.body, theme)
            })
        })
        .collect();

    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| {
            draw_theme_frame(frame, &options, selected, previews[selected].as_ref());
        })?;

        match wait_for_action(options.len(), selected)? {
            Action::Cancel => return Ok(ThemePick::Cancelled),
            Action::Confirm => return Ok(ThemePick::Chose(options[selected].key)),
            Action::Move(to) => selected = to,
            Action::Toggle | Action::Tick => {}
        }
    }
}

fn resolve_theme(key: Option<&'static str>) -> Theme {
    key.and_then(theme_presets::by_name).unwrap_or_default()
}

fn draw_theme_frame(
    frame: &mut Frame,
    options: &[ThemeOption],
    selected: usize,
    preview: Option<&TemplatePreview>,
) {
    let meta = SlotMeta {
        title: "Pick your theme",
        subtitle: "Applied to every splash you render. Override individual tokens later via settings.toml.",
    };
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                        // header
            Constraint::Min(8),                           // preview
            Constraint::Length(options.len() as u16 + 2), // list
            Constraint::Length(1),                        // footer
        ])
        .split(area);

    draw_header(frame, layout[0], &meta, selected + 1, options.len());
    draw_preview(frame, layout[1], options[selected].label, preview);
    draw_theme_list(frame, layout[2], options, selected);
    draw_footer(frame, layout[3], "↑/↓ navigate · Enter select · Esc cancel");
}

fn draw_theme_list(frame: &mut Frame, area: Rect, options: &[ThemeOption], selected: usize) {
    let block = Block::default().borders(Borders::ALL).title(" themes ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = options
        .iter()
        .enumerate()
        .map(|(i, opt)| render_name_desc_row(opt.label, opt.description, i == selected))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Toggle picker ─────────────────────────────────────────────────────────────────

const TOGGLE_BG: usize = 0;
const TOGGLE_WAIT: usize = 1;

fn run_toggle_picker(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<Option<(bool, bool)>> {
    // Defaults match `SettingsChoices::DEFAULT`.
    let mut values = [true, false];
    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| {
            draw_toggle_frame(frame, selected, values);
        })?;
        match wait_for_action(values.len(), selected)? {
            Action::Cancel => return Ok(None),
            Action::Confirm => return Ok(Some((values[TOGGLE_BG], values[TOGGLE_WAIT]))),
            Action::Move(to) => selected = to,
            Action::Toggle => values[selected] = !values[selected],
            Action::Tick => {}
        }
    }
}

fn draw_toggle_frame(frame: &mut Frame, selected: usize, values: [bool; 2]) {
    let meta = SlotMeta {
        title: "Pick your defaults",
        subtitle: "Applied to every splash; edit settings.toml later to change.",
    };
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(4),    // list
            Constraint::Length(1), // footer
        ])
        .split(area);
    draw_header(frame, layout[0], &meta, selected + 1, values.len());

    let block = Block::default().borders(Borders::ALL).title(" options ");
    let inner = block.inner(layout[1]);
    frame.render_widget(block, layout[1]);

    let lines = vec![
        render_toggle_row(
            "paint splash background",
            "off = let the terminal's own background show through (light terminals, transparency)",
            values[TOGGLE_BG],
            selected == TOGGLE_BG,
        ),
        render_toggle_row(
            "wait for fresh data",
            "on = block the first render until every widget has fetched at least once",
            values[TOGGLE_WAIT],
            selected == TOGGLE_WAIT,
        ),
    ];
    frame.render_widget(Paragraph::new(lines), inner);

    draw_footer(
        frame,
        layout[2],
        "↑/↓ navigate · space toggle · Enter continue · Esc cancel",
    );
}

fn render_toggle_row(label: &str, desc: &str, value: bool, active: bool) -> Line<'static> {
    let marker = if active { "▶ " } else { "  " };
    let checkbox = if value { "[x] " } else { "[ ] " };
    let name_style = if active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let desc_style = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(format!("{marker}{checkbox}{label:<26}"), name_style),
        Span::styled(format!("  {desc}"), desc_style),
    ])
}

// ── Shared helpers ────────────────────────────────────────────────────────────────

enum Action {
    Cancel,
    Confirm,
    Toggle,
    Move(usize),
    /// No input came in this tick. Loops keep re-drawing so animations (not used today,
    /// but the preview pipeline supports them) stay visible.
    Tick,
}

fn wait_for_action(total: usize, current: usize) -> io::Result<Action> {
    if !event::poll(Duration::from_millis(250))? {
        return Ok(Action::Tick);
    }
    let Event::Key(key) = event::read()? else {
        return Ok(Action::Tick);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(Action::Tick);
    }
    Ok(match (key.code, key.modifiers) {
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Cancel,
        (KeyCode::Enter, _) => Action::Confirm,
        (KeyCode::Char(' '), _) => Action::Toggle,
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) if current > 0 => Action::Move(current - 1),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) if current + 1 < total => {
            Action::Move(current + 1)
        }
        _ => Action::Tick,
    })
}

fn draw_header(frame: &mut Frame, area: Rect, meta: &SlotMeta, n: usize, total: usize) {
    let title = Line::from(vec![
        Span::styled(
            meta.title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(format!("{n}/{total}"), Style::default().fg(Color::DarkGray)),
    ]);
    let subtitle = Line::from(Span::styled(
        meta.subtitle,
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(
        Paragraph::new(vec![title, subtitle]).alignment(Alignment::Left),
        area,
    );
}

fn draw_preview(frame: &mut Frame, area: Rect, label: &str, preview: Option<&TemplatePreview>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" preview · {label} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match preview {
        Some(p) => p.draw(frame, inner),
        None => {
            let msg = Paragraph::new("(preview unavailable)")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(msg, inner);
        }
    }
}

fn render_name_desc_row(name: &str, desc: &str, active: bool) -> Line<'static> {
    let marker = if active { "▶ " } else { "  " };
    let name_style = if active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let desc_style = Style::default().fg(Color::DarkGray);
    Line::from(vec![
        Span::styled(format!("{marker}{name:<28}"), name_style),
        Span::styled(format!("  {desc}"), desc_style),
    ])
}

fn draw_footer(frame: &mut Frame, area: Rect, hint: &str) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {hint}"),
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}
