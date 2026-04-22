use std::collections::HashMap;
use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;

use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::Paragraph,
};
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheEntry};
use crate::config::{Config, WidgetConfig};
use crate::daemon;
use crate::fetcher::{self, FetchContext, Registry, ShapeMismatch};
use crate::layout::{self, Layout, WidgetId};
use crate::payload::Payload;
use crate::render::{self, RenderSpec, Shape};
use crate::trust::{self, TrustStore};

/// Split trust-passed widgets into cached (disk-backed, fetched async via daemon) and realtime
/// (recomputed on every frame, no disk I/O). Keeps the rest of the runtime simple: cache-aware
/// code only sees cached widgets, per-frame code only sees realtime widgets.
fn split_by_fetcher_kind(
    widgets: &[WidgetConfig],
    registry: &Registry,
) -> (Vec<WidgetConfig>, Vec<WidgetConfig>) {
    widgets
        .iter()
        .cloned()
        .partition(|w| registry.get_cached(&w.fetcher).is_some())
}

/// Derives the shape each widget should ask its fetcher for. Renderers are single-shape today
/// (every builtin's `accepts()` returns one element), so specifying `render = "..."` uniquely
/// determines the shape. Widgets without an explicit renderer fall back to the fetcher's
/// [`fetcher::Fetcher::default_shape`]. Unknown fetcher / renderer names drop out here — we skip
/// them and let the downstream dispatch / render paths surface the mismatch as placeholders.
fn derive_shapes(
    widgets: &[WidgetConfig],
    fetchers: &Registry,
    renderers: &render::Registry,
) -> HashMap<WidgetId, Shape> {
    widgets
        .iter()
        .filter_map(|w| derive_shape(w, fetchers, renderers).map(|s| (w.id.clone(), s)))
        .collect()
}

fn derive_shape(
    w: &WidgetConfig,
    fetchers: &Registry,
    renderers: &render::Registry,
) -> Option<Shape> {
    if let Some(spec) = &w.render
        && let Some(renderer) = renderers.get(spec.renderer_name())
        && let Some(&shape) = renderer.accepts().first()
    {
        return Some(shape);
    }
    fetchers.get(&w.fetcher).map(|f| f.default_shape())
}

/// Partitions widgets into (shape-valid, shape-invalid-with-reason). A widget whose renderer-
/// derived shape isn't in the fetcher's `shapes()` is diverted to a placeholder rather than
/// being dispatched — protects the same "never crash the splash" invariant as unknown-renderer
/// handling in [`render::render_payload`]. Unknown fetcher names pass through; the fetch
/// dispatch drops them separately.
fn partition_by_shape_support(
    widgets: &[WidgetConfig],
    shapes: &HashMap<WidgetId, Shape>,
    fetchers: &Registry,
) -> (Vec<WidgetConfig>, Vec<(WidgetConfig, ShapeMismatch)>) {
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for w in widgets {
        let Some(fetcher) = fetchers.get(&w.fetcher) else {
            valid.push(w.clone());
            continue;
        };
        let Some(&shape) = shapes.get(&w.id) else {
            valid.push(w.clone());
            continue;
        };
        if fetcher.shapes().contains(&shape) {
            valid.push(w.clone());
        } else {
            invalid.push((
                w.clone(),
                ShapeMismatch {
                    fetcher: fetcher.name().to_string(),
                    requested: shape,
                },
            ));
        }
    }
    (valid, invalid)
}

fn compute_realtime_payloads(
    registry: &Registry,
    widgets: &[WidgetConfig],
    shapes: &HashMap<WidgetId, Shape>,
) -> HashMap<WidgetId, Payload> {
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_realtime(&w.fetcher)?;
            let ctx = fetch_context(w, shapes.get(&w.id).copied(), Duration::from_secs(0));
            Some((w.id.clone(), fetcher.compute(&ctx)))
        })
        .collect()
}

const DEFAULT_REFRESH_SECS: u64 = 60;
const FAST_DEADLINE: Duration = Duration::from_millis(1500);
const WAIT_DEADLINE: Duration = Duration::from_secs(5);
const DAEMON_DEADLINE: Duration = Duration::from_secs(30);
/// Frame interval while inside the multi-frame render loop. Animated renderers rely on this
/// cadence to produce motion; sub-50ms would waste CPU, above 100ms would look choppy.
const FRAME_TICK: Duration = Duration::from_millis(50);
/// How long to run the render loop when the only reason for looping is animation (no wait).
/// Long enough for `animated_typewriter` to finish typing; short enough to not noticeably
/// delay the shell on cache-warm startups.
const ANIMATION_WINDOW: Duration = Duration::from_secs(2);
const DEFAULT_VIEWPORT_LINES: u16 = 16;

pub async fn run(
    config: &Config,
    config_ident: Option<(&Path, &str)>,
    wait: bool,
) -> io::Result<()> {
    let layout = config.to_layout();
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let specs = render_specs(&config.widgets);

    // Split widgets into what we can fetch vs what we must gate behind trust. Gated slots render
    // a canned "🔒 requires trust" placeholder so the layout stays intact and the user can see
    // what needs unlocking.
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    // Derive the renderer-implied shape for each widget once, then reuse it in every downstream
    // step (validation, cache key, fetch, realtime compute) so the fetcher sees a single
    // consistent answer.
    let shapes = derive_shapes(&config.widgets, &registry, &render_registry);
    // Shape validation runs on everything the trust gate let through — a widget whose renderer
    // demands a shape the fetcher can't emit renders a placeholder instead of reaching the
    // fetcher.
    let (fetchable, shape_invalid) = partition_by_shape_support(&fetchable, &shapes, &registry);

    // Cached widgets go through disk cache + async daemon. Realtime widgets recompute each
    // frame and never touch disk.
    let (cached_widgets, realtime_widgets) = split_by_fetcher_kind(&fetchable, &registry);

    let entries = load_entries(cache.as_ref(), &registry, &cached_widgets, &shapes);
    let mut payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(
        &registry,
        &realtime_widgets,
        &shapes,
    ));
    for w in &gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for (w, err) in &shape_invalid {
        payloads.insert(w.id.clone(), fetcher::shape_mismatch_placeholder(err));
    }

    // Load axis: decide whether to block on fresh data. If there are any cached widgets with no
    // entry yet, promote to wait so we don't exit with a blank terminal on first run. Realtime
    // widgets don't count — they're always fresh.
    let wait =
        wait || config.general.wait_for_fresh || (!cached_widgets.is_empty() && entries.is_empty());

    // Render axis: decide whether any configured renderer needs repeat frames. Independent of
    // the load decision — an animated widget must animate regardless of cache state.
    let animated = render::any_widget_animates(&config.widgets, &render_registry);

    let loop_window = match (wait, animated) {
        (false, false) => None,
        (false, true) => Some(ANIMATION_WINDOW),
        (true, _) => Some(WAIT_DEADLINE),
    };

    // Default viewport = sum of configured row heights so every widget fits without manual
    // tuning. Users can still override with `general.height` when they want padding or are
    // using Percentage sizes. Clamped to `terminal_rows - 1` so there's always one row left
    // below the viewport for the shell prompt. When the configured height doesn't fit, the
    // bottom row of the visible viewport is sacrificed for a `… +N rows` clip hint so users
    // aren't silently missing widgets.
    let requested_height = config
        .general
        .height
        .unwrap_or_else(|| config.computed_height().max(DEFAULT_VIEWPORT_LINES));
    let term_rows = ratatui::crossterm::terminal::size()
        .map(|(_, h)| h)
        .unwrap_or(requested_height);
    let max_inline = term_rows.saturating_sub(1).max(1);
    let clipping = requested_height > max_inline;
    let viewport_lines = if clipping {
        max_inline
    } else {
        requested_height
    }
    .max(1);
    let hidden_rows = if clipping {
        requested_height.saturating_sub(viewport_lines.saturating_sub(1))
    } else {
        0
    };
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = make_terminal(backend, viewport_lines)?;

    // Cache-first one-shot: no looping needed, paint what we have and exit.
    if loop_window.is_none() {
        draw_final(
            &mut terminal,
            &layout,
            &payloads,
            &specs,
            &render_registry,
            hidden_rows,
        )?;
        let _ = daemon::spawn_fetch_daemon(config_ident.map(|(p, _)| p));
        finalize_splash(&mut terminal);
        return Ok(());
    }

    let window = loop_window.unwrap();
    match daemon::spawn_fetch_daemon(config_ident.map(|(p, _)| p)) {
        Ok(mut child) => {
            // Multi-frame loop: ticks the frame every FRAME_TICK so animated renderers can
            // animate; picks up daemon-written cache entries as they land; stops when the
            // daemon finishes (if we were waiting for it) or the window expires.
            let buckets = WidgetBuckets {
                cached: &cached_widgets,
                gated: &gated,
                shape_invalid: &shape_invalid,
            };
            let deadline = std::time::Instant::now() + window;
            loop {
                refresh_payloads(&cache, &registry, &buckets, &shapes, &mut payloads);
                draw(
                    &mut terminal,
                    &layout,
                    &payloads,
                    &specs,
                    &render_registry,
                    hidden_rows,
                )?;
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    if wait {
                        let _ = child.start_kill();
                    }
                    break;
                }
                let tick = remaining.min(FRAME_TICK);
                if wait {
                    match tokio::time::timeout(tick, child.wait()).await {
                        Ok(_) => break,
                        Err(_) => continue,
                    }
                } else {
                    tokio::time::sleep(tick).await;
                }
            }
            refresh_payloads(&cache, &registry, &buckets, &shapes, &mut payloads);
            draw_final(
                &mut terminal,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                hidden_rows,
            )?;
        }
        Err(_) => {
            // Spawning the daemon failed (OOM, exec permission) — rare. Fall back to inline
            // fetch and a single final draw so the user still sees something useful.
            let fetch_deadline = if wait { WAIT_DEADLINE } else { FAST_DEADLINE };
            let fresh = fetch_all(
                &registry,
                cache.as_ref(),
                &cached_widgets,
                &entries,
                &shapes,
                fetch_deadline,
            )
            .await;
            for (id, payload) in fresh {
                payloads.insert(id, payload);
            }
            // Realtime values in the fallback path are still fresh at this point — the compute
            // above was microseconds ago. No refresh needed before the single final draw.
            draw_final(
                &mut terminal,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                hidden_rows,
            )?;
        }
    }

    finalize_splash(&mut terminal);
    Ok(())
}

/// Runs fetchers and persists the results. Called by the detached `fetch-only` subcommand so the
/// main invocation can exit without waiting for I/O. Errors from individual fetchers are logged
/// nowhere (daemon has no stdio) and simply leave stale cache in place. The daemon re-checks
/// trust itself because it loads config via its own command-line argument, not from shared
/// state.
pub async fn fetch_and_persist(config: &Config, config_ident: Option<(&Path, &str)>) {
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, _gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    let shapes = derive_shapes(&config.widgets, &registry, &render_registry);
    // Shape-invalid widgets are rendered as placeholders by the main process; there's no value
    // in the daemon hitting their fetchers just to have the result discarded.
    let (fetchable, _shape_invalid) = partition_by_shape_support(&fetchable, &shapes, &registry);
    // Daemon only persists cached-fetcher output. Realtime widgets have no disk representation.
    let (cached_widgets, _realtime) = split_by_fetcher_kind(&fetchable, &registry);
    let entries = load_entries(cache.as_ref(), &registry, &cached_widgets, &shapes);
    let _ = fetch_all(
        &registry,
        cache.as_ref(),
        &cached_widgets,
        &entries,
        &shapes,
        DAEMON_DEADLINE,
    )
    .await;
}

/// Widget partitions the runtime carries across refreshes. Bundling keeps
/// [`refresh_payloads`] under clippy's arg-count limit and makes the "what goes where"
/// categories explicit at call sites.
struct WidgetBuckets<'a> {
    cached: &'a [WidgetConfig],
    gated: &'a [WidgetConfig],
    shape_invalid: &'a [(WidgetConfig, ShapeMismatch)],
}

/// Merges fresh daemon-written cache entries into the payload map between frames. Realtime
/// payloads stay exactly as they were computed once before the loop — the animation window is
/// short (2s) and recomputing per frame would make expensive fetchers like `system_processes`
/// scale badly. Trust-gate and shape-mismatch placeholders are re-applied defensively so a
/// daemon-written entry can't leak into a gated slot.
fn refresh_payloads(
    cache: &Option<Cache>,
    registry: &Registry,
    buckets: &WidgetBuckets<'_>,
    shapes: &HashMap<WidgetId, Shape>,
    payloads: &mut HashMap<WidgetId, Payload>,
) {
    let entries = load_entries(cache.as_ref(), registry, buckets.cached, shapes);
    for (id, payload) in entries_to_payloads(&entries) {
        payloads.insert(id, payload);
    }
    for w in buckets.gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for (w, err) in buckets.shape_invalid {
        payloads.insert(w.id.clone(), fetcher::shape_mismatch_placeholder(err));
    }
}

fn fetch_context(w: &WidgetConfig, shape: Option<Shape>, deadline: Duration) -> FetchContext {
    FetchContext {
        widget_id: w.id.clone(),
        format: w.format.clone(),
        timeout: deadline,
        file_format: w.file_format.clone(),
        shape,
        options: w.options.clone(),
    }
}

fn load_entries(
    cache: Option<&Cache>,
    registry: &Registry,
    widgets: &[WidgetConfig],
    shapes: &HashMap<WidgetId, Shape>,
) -> HashMap<WidgetId, CacheEntry> {
    let Some(c) = cache else {
        return HashMap::new();
    };
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_cached(&w.fetcher)?;
            let shape = shapes.get(&w.id).copied();
            let key = fetcher.cache_key(&fetch_context(w, shape, Duration::from_secs(0)));
            c.load(&key).map(|e| (w.id.clone(), e))
        })
        .collect()
}

fn entries_to_payloads(entries: &HashMap<WidgetId, CacheEntry>) -> HashMap<WidgetId, Payload> {
    entries
        .iter()
        .map(|(k, v)| (k.clone(), v.payload.clone()))
        .collect()
}

async fn fetch_all(
    registry: &Registry,
    cache: Option<&Cache>,
    widgets: &[WidgetConfig],
    cached: &HashMap<WidgetId, CacheEntry>,
    shapes: &HashMap<WidgetId, Shape>,
    deadline: Duration,
) -> HashMap<WidgetId, Payload> {
    let mut set = JoinSet::new();
    for w in widgets {
        if !should_refresh(cached, w) {
            continue;
        }
        let Some(fetcher) = registry.get_cached(&w.fetcher) else {
            continue;
        };
        let ctx = fetch_context(w, shapes.get(&w.id).copied(), deadline);
        let cache_key = fetcher.cache_key(&ctx);
        let lock = match cache {
            Some(c) => match c.try_lock(&cache_key) {
                Some(l) => Some(l),
                None => continue,
            },
            None => None,
        };
        let id = w.id.clone();
        let ttl = w.refresh_interval.unwrap_or(DEFAULT_REFRESH_SECS);
        set.spawn(async move {
            let _lock = lock;
            let payload = tokio::time::timeout(deadline, fetcher.fetch(&ctx))
                .await
                .ok()?
                .ok()?;
            Some((id, cache_key, ttl, payload))
        });
    }

    let mut out = HashMap::new();
    while let Some(joined) = set.join_next().await {
        let Ok(Some((id, key, ttl, payload))) = joined else {
            continue;
        };
        if let Some(c) = cache {
            let _ = c.store(&key, &CacheEntry::new(payload.clone(), ttl));
        }
        out.insert(id, payload);
    }
    out
}

fn should_refresh(cached: &HashMap<WidgetId, CacheEntry>, w: &WidgetConfig) -> bool {
    match cached.get(&w.id) {
        Some(e) => !e.is_fresh(),
        None => true,
    }
}

fn make_terminal<B: Backend>(backend: B, viewport_lines: u16) -> io::Result<Terminal<B>> {
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_lines),
        },
    )
}

fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    hidden_rows: u16,
) -> io::Result<()> {
    terminal.draw(|frame| {
        draw_frame(
            frame,
            frame.area(),
            root,
            payloads,
            specs,
            registry,
            hidden_rows,
        )
    })?;
    Ok(())
}

/// Same as [`draw`] but parks the cursor at the bottom-left of the inline area afterwards, so
/// the caller's `println!()` lands one row below the splash instead of inside it. Moves via
/// the backend rather than `Frame::set_cursor_position` — the latter re-shows the cursor and
/// we want it to stay hidden (matches ratatui's default for every intermediate draw). The
/// caller is responsible for re-showing the cursor just before returning to the shell.
fn draw_final<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    hidden_rows: u16,
) -> io::Result<()> {
    let mut captured: Option<Rect> = None;
    terminal.draw(|frame| {
        let area = frame.area();
        captured = Some(area);
        draw_frame(frame, area, root, payloads, specs, registry, hidden_rows);
    })?;
    if let Some(area) = captured
        && area.height > 0
    {
        terminal.set_cursor_position(ratatui::layout::Position {
            x: area.x,
            y: area.y + area.height - 1,
        })?;
    }
    Ok(())
}

/// Renders the layout into the given area, sacrificing the bottom row for a `… +N rows` hint
/// when the configured height doesn't fit and the viewport had to be clamped to the terminal.
fn draw_frame(
    frame: &mut Frame,
    area: Rect,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    hidden_rows: u16,
) {
    if hidden_rows > 0 && area.height > 1 {
        let layout_area = Rect {
            height: area.height - 1,
            ..area
        };
        let hint_area = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        layout::draw(frame, layout_area, root, payloads, specs, registry);
        render_clip_hint(frame, hint_area, hidden_rows);
    } else {
        layout::draw(frame, area, root, payloads, specs, registry);
    }
}

fn render_clip_hint(frame: &mut Frame, area: Rect, hidden_rows: u16) {
    let text = format!("… +{hidden_rows} rows (terminal too short)");
    let style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM | Modifier::ITALIC);
    let p = Paragraph::new(Line::from(text).style(style)).alignment(Alignment::Right);
    frame.render_widget(p, area);
}

/// Hand back to the shell: show the cursor (ratatui hid it during drawing) and emit the
/// trailing newline that moves the cursor past the inline viewport.
fn finalize_splash<B: Backend>(terminal: &mut Terminal<B>) {
    let _ = terminal.show_cursor();
    println!();
}

fn render_specs(widgets: &[WidgetConfig]) -> HashMap<WidgetId, RenderSpec> {
    widgets
        .iter()
        .filter_map(|w| w.render.clone().map(|s| (w.id.clone(), s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout as LayoutTree;
    use crate::payload::{Body, TextData};
    use ratatui::{Terminal, backend::TestBackend};

    fn text_payload(line: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: line.into() }),
        }
    }

    fn single_widget_tree(id: &str) -> LayoutTree {
        LayoutTree::Widget {
            id: id.into(),
            panel: None,
        }
    }

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn draw_frame_hint_lands_on_bottom_row_when_clipping() {
        let root = single_widget_tree("x");
        let mut payloads = HashMap::new();
        payloads.insert("x".into(), text_payload("top"));
        let specs = HashMap::new();
        let registry = render::Registry::with_builtins();
        let backend = TestBackend::new(50, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_frame(f, f.area(), &root, &payloads, &specs, &registry, 7))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let bottom = row_text(&buf, 3);
        assert!(
            bottom.contains("…") && bottom.contains("+7"),
            "expected clip hint in bottom row: {bottom:?}"
        );
        assert!(row_text(&buf, 0).contains("top"), "layout still rendered");
    }

    #[test]
    fn draw_frame_without_clipping_paints_no_hint() {
        let root = single_widget_tree("x");
        let mut payloads = HashMap::new();
        payloads.insert("x".into(), text_payload("only"));
        let specs = HashMap::new();
        let registry = render::Registry::with_builtins();
        let backend = TestBackend::new(50, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| draw_frame(f, f.area(), &root, &payloads, &specs, &registry, 0))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let joined: String = (0..3).map(|y| row_text(&buf, y)).collect();
        assert!(!joined.contains("…"), "no marker expected: {joined:?}");
        assert!(!joined.contains("terminal too short"));
    }

    #[test]
    fn clip_hint_includes_marker_and_count() {
        let backend = TestBackend::new(60, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_clip_hint(f, f.area(), 42))
            .unwrap();
        let row = row_text(&terminal.backend().buffer().clone(), 0);
        assert!(row.contains("…"), "marker missing: {row:?}");
        assert!(row.contains("+42"), "count missing: {row:?}");
    }

    fn widget(id: &str, fetcher: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: fetcher.into(),
            refresh_interval: Some(60),
            ..Default::default()
        }
    }

    #[test]
    fn should_refresh_when_no_cache_entry() {
        let cached = HashMap::new();
        assert!(should_refresh(&cached, &widget("a", "static")));
    }

    #[test]
    fn should_refresh_when_entry_is_stale() {
        let mut cached = HashMap::new();
        cached.insert(
            "a".into(),
            CacheEntry {
                refreshed_at: 0,
                ttl_seconds: 60,
                payload: text_payload("x"),
            },
        );
        assert!(should_refresh(&cached, &widget("a", "static")));
    }

    #[test]
    fn should_skip_when_entry_is_fresh() {
        let mut cached = HashMap::new();
        cached.insert("a".into(), CacheEntry::new(text_payload("x"), 60));
        assert!(!should_refresh(&cached, &widget("a", "static")));
    }

    fn static_widget(id: &str, text: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: "static".into(),
            format: Some(text.into()),
            refresh_interval: Some(60),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn fetch_all_populates_cache_and_returns_fresh_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let widgets = vec![static_widget("greeting", "Hi!")];

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
            &HashMap::new(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        assert!(fresh.contains_key("greeting"));
        let key = registry
            .get_cached("static")
            .unwrap()
            .cache_key(&fetch_context(&widgets[0], None, Duration::from_secs(0)));
        let loaded = cache.load(&key).unwrap();
        match loaded.payload.body {
            Body::TextBlock(t) => assert_eq!(t.lines, vec!["Hi!".to_string()]),
            _ => panic!("expected text_block body"),
        }
    }

    #[tokio::test]
    async fn widgets_sharing_fetcher_and_params_share_cache_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let widgets = [
            static_widget("greeting_project_a", "Hello!"),
            static_widget("greeting_project_b", "Hello!"),
        ];

        let _ = fetch_all(
            &registry,
            Some(&cache),
            &widgets[..1],
            &HashMap::new(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        let entries = load_entries(Some(&cache), &registry, &widgets[1..], &HashMap::new());
        assert!(entries.contains_key("greeting_project_b"));
    }

    #[tokio::test]
    async fn widgets_with_different_params_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let first = vec![static_widget("a", "one")];
        let second = vec![static_widget("b", "two")];

        let _ = fetch_all(
            &registry,
            Some(&cache),
            &first,
            &HashMap::new(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        let entries = load_entries(Some(&cache), &registry, &second, &HashMap::new());
        assert!(
            entries.is_empty(),
            "second widget should not read first widget's cache"
        );
    }

    #[tokio::test]
    async fn fetch_all_skips_fresh_cached_widgets() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let widgets = vec![WidgetConfig {
            id: "greeting".into(),
            fetcher: "static".into(),
            format: Some("Hi!".into()),
            refresh_interval: Some(60),
            ..Default::default()
        }];
        let mut cached = HashMap::new();
        cached.insert("greeting".into(), CacheEntry::new(text_payload("old"), 60));

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
            &cached,
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        assert!(fresh.is_empty());
    }

    #[tokio::test]
    async fn fetch_all_skips_widgets_whose_key_is_already_locked() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let widgets = vec![static_widget("greeting", "Hi!")];
        let key = registry
            .get_cached("static")
            .unwrap()
            .cache_key(&fetch_context(&widgets[0], None, Duration::from_secs(0)));
        let _held = cache.try_lock(&key).expect("first acquire");

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
            &HashMap::new(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        assert!(
            fresh.is_empty(),
            "held lock should cause the fetch to be skipped"
        );
        assert!(
            cache.load(&key).is_none(),
            "no cache should be written when fetch is skipped"
        );
    }

    #[tokio::test]
    async fn fetch_all_ignores_unknown_fetcher() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget("x", "no_such_fetcher")];
        let fresh = fetch_all(
            &registry,
            None,
            &widgets,
            &HashMap::new(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;
        assert!(fresh.is_empty());
    }

    fn widget_with_render(id: &str, fetcher: &str, renderer: Option<&str>) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: fetcher.into(),
            render: renderer.map(|r| RenderSpec::Short(r.into())),
            ..Default::default()
        }
    }

    fn single_shape(id: &str, shape: Shape) -> HashMap<WidgetId, Shape> {
        let mut m = HashMap::new();
        m.insert(id.into(), shape);
        m
    }

    #[test]
    fn derive_shape_prefers_renderer_accepted_shape_over_fetcher_default() {
        let registry = Registry::with_builtins();
        let render_registry = render::Registry::with_builtins();
        let w = widget_with_render("t", "clock", Some("grid_calendar"));
        assert_eq!(
            derive_shape(&w, &registry, &render_registry),
            Some(Shape::Calendar)
        );
    }

    #[test]
    fn derive_shape_falls_back_to_fetcher_default_when_no_render_spec() {
        let registry = Registry::with_builtins();
        let render_registry = render::Registry::with_builtins();
        let w = widget_with_render("t", "clock", None);
        assert_eq!(
            derive_shape(&w, &registry, &render_registry),
            Some(Shape::Text)
        );
    }

    #[test]
    fn derive_shape_falls_back_to_fetcher_default_when_renderer_unknown() {
        let registry = Registry::with_builtins();
        let render_registry = render::Registry::with_builtins();
        let w = widget_with_render("t", "clock", Some("definitely_not_a_renderer"));
        assert_eq!(
            derive_shape(&w, &registry, &render_registry),
            Some(Shape::Text)
        );
    }

    #[test]
    fn partition_by_shape_support_keeps_widget_with_compatible_renderer() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget_with_render("d", "disk_usage", Some("gauge_circle"))];
        let shapes = single_shape("d", Shape::Ratio);
        let (valid, invalid) = partition_by_shape_support(&widgets, &shapes, &registry);
        assert_eq!(valid.len(), 1);
        assert!(invalid.is_empty());
    }

    #[test]
    fn partition_by_shape_support_rejects_incompatible_renderer() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget_with_render("d", "disk_usage", Some("grid_calendar"))];
        let shapes = single_shape("d", Shape::Calendar);
        let (valid, invalid) = partition_by_shape_support(&widgets, &shapes, &registry);
        assert!(valid.is_empty());
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid[0].0.id, "d");
        assert_eq!(invalid[0].1.requested, Shape::Calendar);
    }

    #[test]
    fn partition_by_shape_support_passes_through_unknown_fetcher() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget_with_render("x", "no_such_fetcher", Some("grid_calendar"))];
        let shapes = single_shape("x", Shape::Calendar);
        let (valid, invalid) = partition_by_shape_support(&widgets, &shapes, &registry);
        assert_eq!(valid.len(), 1);
        assert!(invalid.is_empty());
    }

    #[test]
    fn partition_by_shape_support_accepts_multi_shape_fetchers_renderer_shape() {
        let registry = Registry::with_builtins();
        let widgets = vec![
            widget_with_render("a", "clock", Some("text_plain")),
            widget_with_render("b", "clock", Some("grid_table")),
            widget_with_render("c", "clock", Some("grid_calendar")),
        ];
        let mut shapes = HashMap::new();
        shapes.insert("a".into(), Shape::Text);
        shapes.insert("b".into(), Shape::Entries);
        shapes.insert("c".into(), Shape::Calendar);
        let (valid, invalid) = partition_by_shape_support(&widgets, &shapes, &registry);
        assert_eq!(valid.len(), 3);
        assert!(invalid.is_empty());
    }
}
