use std::collections::HashMap;
use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;

use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend},
};
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheEntry};
use crate::config::{Config, WidgetConfig};
use crate::daemon;
use crate::fetcher::{FetchContext, Registry};
use crate::layout::{self, Layout, WidgetId};
use crate::payload::Payload;
use crate::render::{self, RenderSpec};
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

fn compute_realtime_payloads(
    registry: &Registry,
    widgets: &[WidgetConfig],
) -> HashMap<WidgetId, Payload> {
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_realtime(&w.fetcher)?;
            let ctx = fetch_context(w, Duration::from_secs(0));
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

    // Cached widgets go through disk cache + async daemon. Realtime widgets recompute each
    // frame and never touch disk.
    let (cached_widgets, realtime_widgets) = split_by_fetcher_kind(&fetchable, &registry);

    let entries = load_entries(cache.as_ref(), &registry, &cached_widgets);
    let mut payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(&registry, &realtime_widgets));
    for w in &gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
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
    // below the viewport for the shell prompt to land on cleanly.
    let requested_height = config
        .general
        .height
        .unwrap_or_else(|| config.computed_height().max(DEFAULT_VIEWPORT_LINES));
    let term_rows = ratatui::crossterm::terminal::size()
        .map(|(_, h)| h)
        .unwrap_or(requested_height);
    let viewport_lines = requested_height.min(term_rows.saturating_sub(1)).max(1);
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = make_terminal(backend, viewport_lines)?;

    // Cache-first one-shot: no looping needed, paint what we have and exit.
    if loop_window.is_none() {
        draw_final(&mut terminal, &layout, &payloads, &specs, &render_registry)?;
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
            let deadline = std::time::Instant::now() + window;
            loop {
                refresh_payloads(
                    &cache,
                    &registry,
                    &cached_widgets,
                    &realtime_widgets,
                    &gated,
                    &mut payloads,
                );
                draw(&mut terminal, &layout, &payloads, &specs, &render_registry)?;
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
            refresh_payloads(
                &cache,
                &registry,
                &cached_widgets,
                &realtime_widgets,
                &gated,
                &mut payloads,
            );
            draw_final(&mut terminal, &layout, &payloads, &specs, &render_registry)?;
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
                fetch_deadline,
            )
            .await;
            for (id, payload) in fresh {
                payloads.insert(id, payload);
            }
            // Realtime values in the fallback path are still fresh at this point — the compute
            // above was microseconds ago. No refresh needed before the single final draw.
            draw_final(&mut terminal, &layout, &payloads, &specs, &render_registry)?;
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
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, _gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    // Daemon only persists cached-fetcher output. Realtime widgets have no disk representation.
    let (cached_widgets, _realtime) = split_by_fetcher_kind(&fetchable, &registry);
    let entries = load_entries(cache.as_ref(), &registry, &cached_widgets);
    let _ = fetch_all(
        &registry,
        cache.as_ref(),
        &cached_widgets,
        &entries,
        DAEMON_DEADLINE,
    )
    .await;
}

/// Rebuilds the payload map after the daemon has run: pulls fresh entries from the cache for
/// cached widgets, recomputes realtime widgets, and re-applies the trust-gate placeholders on
/// top. Used by both the `--wait` path and the first-run fallback so the two stay in sync.
fn refresh_payloads(
    cache: &Option<Cache>,
    registry: &Registry,
    cached_widgets: &[WidgetConfig],
    realtime_widgets: &[WidgetConfig],
    gated: &[WidgetConfig],
    payloads: &mut HashMap<WidgetId, Payload>,
) {
    let entries = load_entries(cache.as_ref(), registry, cached_widgets);
    *payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(registry, realtime_widgets));
    for w in gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
}

fn fetch_context(w: &WidgetConfig, deadline: Duration) -> FetchContext {
    FetchContext {
        widget_id: w.id.clone(),
        format: w.format.clone(),
        timeout: deadline,
        file_format: w.file_format.clone(),
        shape: w.shape.clone(),
    }
}

fn load_entries(
    cache: Option<&Cache>,
    registry: &Registry,
    widgets: &[WidgetConfig],
) -> HashMap<WidgetId, CacheEntry> {
    let Some(c) = cache else {
        return HashMap::new();
    };
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_cached(&w.fetcher)?;
            let key = fetcher.cache_key(&fetch_context(w, Duration::from_secs(0)));
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
        let ctx = fetch_context(w, deadline);
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
) -> io::Result<()> {
    terminal.draw(|frame| layout::draw(frame, frame.area(), root, payloads, specs, registry))?;
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
) -> io::Result<()> {
    let mut captured: Option<ratatui::layout::Rect> = None;
    terminal.draw(|frame| {
        let area = frame.area();
        captured = Some(area);
        layout::draw(frame, area, root, payloads, specs, registry);
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
    use crate::payload::{Body, LinesData};

    fn text_payload(line: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Lines(LinesData {
                lines: vec![line.into()],
            }),
        }
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
            Duration::from_secs(1),
        )
        .await;

        assert!(fresh.contains_key("greeting"));
        let key = registry
            .get_cached("static")
            .unwrap()
            .cache_key(&fetch_context(&widgets[0], Duration::from_secs(0)));
        let loaded = cache.load(&key).unwrap();
        match loaded.payload.body {
            Body::Lines(t) => assert_eq!(t.lines, vec!["Hi!".to_string()]),
            _ => panic!("expected text body"),
        }
    }

    #[tokio::test]
    async fn widgets_sharing_fetcher_and_params_share_cache_file() {
        // Two different widget ids, same fetcher + format → one cache entry, readable by either.
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let widgets = [
            static_widget("greeting_project_a", "Hello!"),
            static_widget("greeting_project_b", "Hello!"),
        ];

        // First widget populates the cache.
        let _ = fetch_all(
            &registry,
            Some(&cache),
            &widgets[..1],
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        // Second widget sees the shared entry when we look it up by its cache key.
        let entries = load_entries(Some(&cache), &registry, &widgets[1..]);
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
            Duration::from_secs(1),
        )
        .await;

        // Same widget id but different format should not be served the other's cache.
        let entries = load_entries(Some(&cache), &registry, &second);
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
            Duration::from_secs(1),
        )
        .await;

        assert!(fresh.is_empty());
    }

    #[tokio::test]
    async fn fetch_all_skips_widgets_whose_key_is_already_locked() {
        // Simulates a second splashboard invocation arriving while the first still holds the
        // fetch lock for the same cache key — the second should skip rather than duplicate the
        // request. We hold the lock manually here to stand in for the "first" process.
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let registry = Registry::with_builtins();
        let widgets = vec![static_widget("greeting", "Hi!")];
        let key = registry
            .get_cached("static")
            .unwrap()
            .cache_key(&fetch_context(&widgets[0], Duration::from_secs(0)));
        let _held = cache.try_lock(&key).expect("first acquire");

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
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
            Duration::from_secs(1),
        )
        .await;
        assert!(fresh.is_empty());
    }
}
