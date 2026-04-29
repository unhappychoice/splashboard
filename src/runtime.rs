use std::collections::HashMap;
use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;

use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::{Backend, CrosstermBackend, TestBackend},
    buffer::Buffer,
    layout::{Alignment, Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Paragraph},
};
use tokio::task::JoinSet;

use crate::cache::{Cache, CacheEntry, CacheEntryKind};
use crate::config::{Config, General, WidgetConfig};
use crate::daemon;
use crate::fetcher::{self, FetchContext, Registry, ShapeMismatch};
use crate::layout::{self, Layout, WidgetId};
use crate::payload::Payload;
use crate::render::{self, RenderSpec, Shape};
use crate::theme::Theme;
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
pub fn derive_shapes(
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
    {
        let accepted = renderer.accepts();
        // Prefer the fetcher's own shape ordering intersected with what the renderer
        // accepts — fetchers list their most-natural shape first (e.g. `project` lists
        // `TextBlock` ahead of `Text`), and that preference should survive even when the
        // renderer accepts multiple shapes. Falls back to the renderer's first accept when
        // the fetcher is unknown or has no intersection, so the old "first accept wins"
        // behaviour still holds for placeholder / mismatch paths.
        if let Some(fetcher) = fetchers.get(&w.fetcher)
            && let Some(shape) = fetcher.shapes().into_iter().find(|s| accepted.contains(s))
        {
            return Some(shape);
        }
        if let Some(&shape) = accepted.first() {
            return Some(shape);
        }
    }
    fetchers.get(&w.fetcher).map(|f| f.default_shape())
}

/// Partitions widgets into (known-fetcher, unknown-fetcher). Unknown fetcher names — typo,
/// renamed widget, or a config newer than the installed binary — would otherwise drop silently
/// in the realtime/cached dispatch and leave the slot blank. Diverting them at the runtime
/// boundary mirrors the renderer side of the contract: an unknown name renders an in-band
/// `⚠ unknown fetcher: <name>` placeholder so the user can see what to fix.
fn partition_by_known_fetcher(
    widgets: &[WidgetConfig],
    registry: &Registry,
) -> (Vec<WidgetConfig>, Vec<WidgetConfig>) {
    widgets
        .iter()
        .cloned()
        .partition(|w| registry.get(&w.fetcher).is_some())
}

/// Partitions widgets into (shape-valid, shape-invalid-with-reason). A widget whose renderer-
/// derived shape isn't in the fetcher's `shapes()` is diverted to a placeholder rather than
/// being dispatched — protects the same "never crash the splash" invariant as unknown-renderer
/// handling in [`render::render_payload`]. Unknown fetcher names are filtered out before this
/// runs, so the unknown-fetcher branch is a defensive no-op.
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
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
) -> HashMap<WidgetId, Payload> {
    widgets
        .iter()
        .filter_map(|w| {
            let fetcher = registry.get_realtime(&w.fetcher)?;
            let ctx = fetch_context(
                w,
                general,
                shapes.get(&w.id).copied(),
                Duration::from_secs(0),
            );
            Some((w.id.clone(), fetcher.compute(&ctx)))
        })
        .collect()
}

/// Widgets whose data we're still waiting on. Two cases are both treated as "loading":
///
/// 1. No payload at all (first run, cleared cache, brand-new widget) — always loading.
/// 2. Under `--wait`, a stale cache entry whose TTL has expired — the daemon is on the
///    critical path and will refetch, so the displayed payload is about to change. Showing a
///    spinner telegraphs that intent instead of rendering last run's data as if it were
///    authoritative. Without `--wait` we prefer the stale payload to a spinner because the
///    refetch is best-effort (background daemon) and may not land in this invocation.
///
/// Recomputed each iteration so the spinner is swapped off the moment the daemon fills or
/// refreshes the slot. Shape is carried through so future per-shape skeletons can branch on
/// it in `render_loading`.
fn compute_loading(
    cached: &[WidgetConfig],
    payloads: &HashMap<WidgetId, Payload>,
    entries: &HashMap<WidgetId, CacheEntry>,
    shapes: &HashMap<WidgetId, Shape>,
    wait: bool,
) -> HashMap<WidgetId, Shape> {
    cached
        .iter()
        .filter(|w| is_loading(w, payloads, entries, wait))
        .filter_map(|w| shapes.get(&w.id).copied().map(|s| (w.id.clone(), s)))
        .collect()
}

fn is_loading(
    w: &WidgetConfig,
    payloads: &HashMap<WidgetId, Payload>,
    entries: &HashMap<WidgetId, CacheEntry>,
    wait: bool,
) -> bool {
    if !payloads.contains_key(&w.id) {
        return true;
    }
    wait && entries.get(&w.id).is_some_and(|e| !e.is_fresh())
}

fn has_loading_widget(
    cached: &[WidgetConfig],
    payloads: &HashMap<WidgetId, Payload>,
    entries: &HashMap<WidgetId, CacheEntry>,
    wait: bool,
) -> bool {
    cached
        .iter()
        .any(|w| is_loading(w, payloads, entries, wait))
}

/// Loop exit condition factored out so the "wait for both load and animation axes to settle"
/// invariant can be tested without spinning up a real daemon. Matches the inline check in
/// `run()` exactly — hitting the hard deadline always exits, otherwise both the daemon
/// (load axis) and the animation (render axis) must be done.
fn loop_should_exit(daemon_done: bool, animation_pending: bool, deadline_reached: bool) -> bool {
    deadline_reached || (daemon_done && !animation_pending)
}

/// Whether the loop should still be holding for the animation window. `animation_due` is the
/// animation deadline check (`now < deadline`), `real_animated` is true when a configured
/// renderer genuinely wants to animate, `loading_pending` is true when at least one cached
/// widget still has no fresh payload. The animation window is only *binding* while something
/// needs to animate — loading spinners are the common case for minimal presets, and once the
/// daemon populates their cache entries the window stops blocking exit.
fn animation_still_due(animation_due: bool, real_animated: bool, loading_pending: bool) -> bool {
    animation_due && (real_animated || loading_pending)
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
    source: &crate::config::DashboardSource,
    config_ident: Option<(&Path, &str)>,
    wait: bool,
) -> io::Result<()> {
    let layout = config.to_layout();
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let render_registry = render::Registry::with_builtins();
    let specs = render_specs(&config.widgets);
    let theme = Theme::from_config(&config.theme);
    let padding = config.general.padding.map(|p| p.xy()).unwrap_or((0, 0));

    // Split widgets into what we can fetch vs what we must gate behind trust. Gated slots render
    // a canned "🔒 requires trust" placeholder so the layout stays intact and the user can see
    // what needs unlocking.
    let decision = TrustStore::load().decide(config_ident);
    let (fetchable, gated) = trust::partition_by_trust(&config.widgets, &registry, decision);
    // Unknown fetcher names get diverted here so the silent-drop sites downstream
    // (compute_realtime_payloads / fetch_all / load_entries) only ever see registered names.
    let (fetchable, unknown) = partition_by_known_fetcher(&fetchable, &registry);
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

    let entries = load_entries(
        cache.as_ref(),
        &registry,
        &cached_widgets,
        &config.general,
        &shapes,
    );
    let mut payloads = entries_to_payloads(&entries);
    payloads.extend(compute_realtime_payloads(
        &registry,
        &realtime_widgets,
        &config.general,
        &shapes,
    ));
    for w in &gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for w in &unknown {
        payloads.insert(
            w.id.clone(),
            fetcher::unknown_fetcher_placeholder(&w.fetcher),
        );
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
    // the load decision — an animated widget must animate regardless of cache state. Loading
    // placeholders also animate (spinner glyph) so we force-enable animation whenever a cached
    // widget has no entry yet — otherwise the spinner would appear to freeze mid-draw. We keep
    // the two reasons separate: `real_animated` forces the loop to run the full ANIMATION_WINDOW
    // so a configured effect plays; loading-only animation is dropped the moment the daemon's
    // fresh entries clear the last spinner (checked per iteration inside the loop).
    let real_animated = render::any_widget_animates(&config.widgets, &render_registry);
    let animated = real_animated || has_loading_widget(&cached_widgets, &payloads, &entries, wait);

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
        let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
        finalize_draw(
            &mut terminal,
            &layout,
            &payloads,
            &specs,
            &render_registry,
            &theme,
            &config.general,
            padding,
            requested_height,
            viewport_lines,
            &loading,
        )?;
        let _ = daemon::spawn_fetch_daemon(source);
        finalize_splash(&mut terminal);
        return Ok(());
    }

    let window = loop_window.unwrap();
    match daemon::spawn_fetch_daemon(source) {
        Ok(mut child) => {
            // Multi-frame loop: ticks the frame every FRAME_TICK so animated renderers can
            // animate; picks up daemon-written cache entries as they land; stops when the
            // daemon finishes (if we were waiting for it) or the window expires.
            let buckets = WidgetBuckets {
                cached: &cached_widgets,
                gated: &gated,
                unknown: &unknown,
                shape_invalid: &shape_invalid,
            };
            let start = std::time::Instant::now();
            let deadline = start + window;
            let animation_deadline = animated.then(|| start + ANIMATION_WINDOW);
            // `wait = true` means the daemon is on the critical path; flip this once it exits
            // so later iterations skip the `child.wait()` branch. The load-axis and render-axis
            // exit conditions are independent — break only when both are satisfied, or when the
            // hard window deadline expires.
            let mut daemon_done = !wait;
            loop {
                let entries = refresh_payloads(
                    &cache,
                    &registry,
                    &buckets,
                    &config.general,
                    &shapes,
                    &mut payloads,
                );
                let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
                draw(
                    &mut terminal,
                    &layout,
                    &payloads,
                    &specs,
                    &render_registry,
                    &theme,
                    &config.general,
                    padding,
                    hidden_rows,
                    &loading,
                )?;
                let now = std::time::Instant::now();
                let remaining = deadline.saturating_duration_since(now);
                if remaining.is_zero() {
                    if !daemon_done {
                        let _ = child.start_kill();
                    }
                    break;
                }
                // Hold the loop for the animation window only while there's something that
                // actually needs to animate: either a configured animated renderer or a
                // still-loading spinner. A minimal preset on a cold cache sets `animated = true`
                // purely because of spinners; once the daemon writes their entries the loading
                // set empties and we can exit as soon as the daemon signals done, instead of
                // burning the remainder of the 2s window for nothing.
                let animation_pending = animation_still_due(
                    animation_deadline.is_some_and(|d| now < d),
                    real_animated,
                    !loading.is_empty(),
                );
                if loop_should_exit(daemon_done, animation_pending, false) {
                    break;
                }
                let tick = remaining.min(FRAME_TICK);
                if !daemon_done {
                    if tokio::time::timeout(tick, child.wait()).await.is_ok() {
                        daemon_done = true;
                    }
                } else {
                    tokio::time::sleep(tick).await;
                }
            }
            let entries = refresh_payloads(
                &cache,
                &registry,
                &buckets,
                &config.general,
                &shapes,
                &mut payloads,
            );
            let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
            finalize_draw(
                &mut terminal,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                &theme,
                &config.general,
                padding,
                requested_height,
                viewport_lines,
                &loading,
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
                &config.general,
                &shapes,
                fetch_deadline,
            )
            .await;
            for (id, payload) in fresh {
                payloads.insert(id, payload);
            }
            // Realtime values in the fallback path are still fresh at this point — the compute
            // above was microseconds ago. No refresh needed before the single final draw.
            let loading = compute_loading(&cached_widgets, &payloads, &entries, &shapes, wait);
            finalize_draw(
                &mut terminal,
                &layout,
                &payloads,
                &specs,
                &render_registry,
                &theme,
                &config.general,
                padding,
                requested_height,
                viewport_lines,
                &loading,
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
    // Unknown fetcher names are surfaced by the main process as placeholders; the daemon has
    // nothing to fetch for them.
    let (fetchable, _unknown) = partition_by_known_fetcher(&fetchable, &registry);
    let shapes = derive_shapes(&config.widgets, &registry, &render_registry);
    // Shape-invalid widgets are rendered as placeholders by the main process; there's no value
    // in the daemon hitting their fetchers just to have the result discarded.
    let (fetchable, _shape_invalid) = partition_by_shape_support(&fetchable, &shapes, &registry);
    // Daemon only persists cached-fetcher output. Realtime widgets have no disk representation.
    let (cached_widgets, _realtime) = split_by_fetcher_kind(&fetchable, &registry);
    let entries = load_entries(
        cache.as_ref(),
        &registry,
        &cached_widgets,
        &config.general,
        &shapes,
    );
    let _ = fetch_all(
        &registry,
        cache.as_ref(),
        &cached_widgets,
        &entries,
        &config.general,
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
    unknown: &'a [WidgetConfig],
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
    general: &General,
    shapes: &HashMap<WidgetId, Shape>,
    payloads: &mut HashMap<WidgetId, Payload>,
) -> HashMap<WidgetId, CacheEntry> {
    let entries = load_entries(cache.as_ref(), registry, buckets.cached, general, shapes);
    for (id, payload) in entries_to_payloads(&entries) {
        payloads.insert(id, payload);
    }
    for w in buckets.gated {
        payloads.insert(w.id.clone(), trust::requires_trust_placeholder());
    }
    for w in buckets.unknown {
        payloads.insert(
            w.id.clone(),
            fetcher::unknown_fetcher_placeholder(&w.fetcher),
        );
    }
    for (w, err) in buckets.shape_invalid {
        payloads.insert(w.id.clone(), fetcher::shape_mismatch_placeholder(err));
    }
    entries
}

fn fetch_context(
    w: &WidgetConfig,
    general: &General,
    shape: Option<Shape>,
    deadline: Duration,
) -> FetchContext {
    FetchContext {
        widget_id: w.id.clone(),
        format: w.format.clone(),
        timeout: deadline,
        file_format: w.file_format.clone(),
        shape,
        options: w.options.clone(),
        timezone: general.timezone.clone(),
        locale: general.locale.clone(),
    }
}

fn load_entries(
    cache: Option<&Cache>,
    registry: &Registry,
    widgets: &[WidgetConfig],
    general: &General,
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
            let key = fetcher.cache_key(&fetch_context(w, general, shape, Duration::from_secs(0)));
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

/// Short TTL for error/timeout cache entries so a transient blip doesn't pin the widget on the
/// warning placeholder for long. Set well below the 60s default refresh so the next tick
/// retries; long enough that back-to-back renders don't re-run a fetch that just failed.
const ERROR_CACHE_TTL_SECS: u64 = 30;

async fn fetch_all(
    registry: &Registry,
    cache: Option<&Cache>,
    widgets: &[WidgetConfig],
    cached: &HashMap<WidgetId, CacheEntry>,
    general: &General,
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
        let ctx = fetch_context(w, general, shapes.get(&w.id).copied(), deadline);
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
        let fetcher_name = fetcher.name().to_string();
        set.spawn(async move {
            let _lock = lock;
            match tokio::time::timeout(deadline, fetcher.fetch(&ctx)).await {
                Ok(Ok(payload)) => (id, cache_key, ttl, CacheEntryKind::Ok, payload),
                Ok(Err(e)) => {
                    tracing::error!(
                        widget = %id,
                        fetcher = %fetcher_name,
                        cache_key = %cache_key,
                        error = %e,
                        "fetch failed"
                    );
                    (
                        id,
                        cache_key,
                        ERROR_CACHE_TTL_SECS,
                        CacheEntryKind::Err,
                        fetcher::error_placeholder(&e.to_string()),
                    )
                }
                Err(_) => {
                    tracing::warn!(
                        widget = %id,
                        fetcher = %fetcher_name,
                        cache_key = %cache_key,
                        timeout_ms = deadline.as_millis() as u64,
                        "fetch timed out"
                    );
                    (
                        id,
                        cache_key,
                        ERROR_CACHE_TTL_SECS,
                        CacheEntryKind::Timeout,
                        fetcher::timeout_placeholder(),
                    )
                }
            }
        });
    }

    let mut out = HashMap::new();
    while let Some(joined) = set.join_next().await {
        let Ok((id, key, ttl, kind, payload)) = joined else {
            continue;
        };
        if let Some(c) = cache {
            let _ = c.store(&key, &CacheEntry::new_with_kind(payload.clone(), ttl, kind));
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

fn make_terminal<B: Backend>(backend: B, viewport_lines: u16) -> Result<Terminal<B>, B::Error> {
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(viewport_lines),
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    hidden_rows: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    terminal.draw(|frame| {
        draw_frame(
            frame,
            frame.area(),
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            padding,
            hidden_rows,
            loading,
        )
    })?;
    Ok(())
}

/// Same as [`draw`] but parks the cursor at the bottom-left of the inline area afterwards, so
/// the caller's `println!()` lands one row below the splash instead of inside it. Moves via
/// the backend rather than `Frame::set_cursor_position` — the latter re-shows the cursor and
/// we want it to stay hidden (matches ratatui's default for every intermediate draw). The
/// caller is responsible for re-showing the cursor just before returning to the shell.
#[allow(clippy::too_many_arguments)]
fn draw_final<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    hidden_rows: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    let mut captured: Option<Rect> = None;
    terminal.draw(|frame| {
        let area = frame.area();
        captured = Some(area);
        draw_frame(
            frame,
            area,
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            padding,
            hidden_rows,
            loading,
        );
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

/// End-of-run draw: when the configured height doesn't fit the terminal, flush the top rows
/// into scrollback via [`Terminal::insert_before`] and repaint the viewport with the bottom
/// slice — no clip hint. This way the animation plays in the clamped viewport, then scrolls up
/// at exit so the full splash is visible (bottom on screen, rest scrollable). When the layout
/// fits, falls through to the regular [`draw_final`].
#[allow(clippy::too_many_arguments)]
fn finalize_draw<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    requested_height: u16,
    viewport_lines: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    let scrollback_rows = requested_height.saturating_sub(viewport_lines);
    if scrollback_rows == 0 {
        return draw_final(
            terminal, root, payloads, specs, registry, theme, general, padding, 0, loading,
        );
    }
    flush_full_to_scrollback(
        terminal,
        root,
        payloads,
        specs,
        registry,
        theme,
        general,
        padding,
        requested_height,
        viewport_lines,
        loading,
    )
}

#[allow(clippy::too_many_arguments)]
fn flush_full_to_scrollback<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    requested_height: u16,
    viewport_lines: u16,
    loading: &HashMap<WidgetId, Shape>,
) -> Result<(), B::Error> {
    let width = terminal.size()?.width;
    let full_buf = render_full_into_buffer(
        width,
        requested_height,
        root,
        payloads,
        specs,
        registry,
        theme,
        general,
        padding,
        loading,
    );
    let scrollback_rows = requested_height - viewport_lines;
    terminal.insert_before(scrollback_rows, |buf| {
        copy_rows(&full_buf, 0..scrollback_rows, buf);
    })?;
    let mut captured: Option<Rect> = None;
    terminal.draw(|frame| {
        let area = frame.area();
        captured = Some(area);
        copy_rows(
            &full_buf,
            scrollback_rows..requested_height,
            frame.buffer_mut(),
        );
    })?;
    if let Some(area) = captured
        && area.height > 0
    {
        terminal.set_cursor_position(Position {
            x: area.x,
            y: area.y + area.height - 1,
        })?;
    }
    Ok(())
}

/// Renders the layout into an off-screen buffer at full configured height. Uses `TestBackend`
/// only because it's ratatui's one publicly exposed in-memory `Backend` — the name is
/// historical, nothing about it is test-only.
#[allow(clippy::too_many_arguments)]
fn render_full_into_buffer(
    width: u16,
    height: u16,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    loading: &HashMap<WidgetId, Shape>,
) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut inner = Terminal::new(backend).expect("TestBackend::new is infallible");
    inner
        .draw(|frame| {
            // `TestBackend` stores cell symbols verbatim and never interprets escape
            // sequences, so the OSC 8 wrap that `list_links` adds for clickable hyperlinks
            // would only pollute this off-screen buffer (the diff drops the wrapped row's
            // skip cells, leaving them empty in the captured scrollback). Suppress the wrap
            // for this draw — the user-facing on-screen frame still goes through
            // `CrosstermBackend` without suppression and keeps the clickable links.
            render::list_links::without_osc8(|| {
                let area = frame.area();
                paint_viewport_bg(frame, area, theme);
                let content = shrink(area, padding);
                layout::draw(
                    frame, content, root, payloads, specs, registry, theme, general, loading,
                );
            });
        })
        .expect("TestBackend draw is infallible");
    inner.backend().buffer().clone()
}

/// Copies a vertical slice from `src` (rows `src_y_range`, taken relative to `src.area.top()`)
/// into `dst`, laid out from `dst.area.top()` onward. Widths are clamped to the narrower of the
/// two buffers. Each destination row uses the same column origin as `dst.area.left()` so it
/// works for both an `insert_before` buffer (origin `(viewport_x, 0)`) and a frame buffer
/// (origin `(viewport_x, viewport_y)`).
fn copy_rows(src: &Buffer, src_y_range: std::ops::Range<u16>, dst: &mut Buffer) {
    let src_area = src.area;
    let dst_area = dst.area;
    let copy_width = src_area.width.min(dst_area.width);
    for (i, offset) in src_y_range.enumerate() {
        let src_y = src_area.top() + offset;
        let dst_y = dst_area.top() + i as u16;
        if dst_y >= dst_area.bottom() {
            break;
        }
        for x in 0..copy_width {
            let src_pos = Position::new(src_area.left() + x, src_y);
            let dst_pos = Position::new(dst_area.left() + x, dst_y);
            if let (Some(src_cell), Some(dst_cell)) = (src.cell(src_pos), dst.cell_mut(dst_pos)) {
                *dst_cell = src_cell.clone();
            }
        }
    }
}

/// Renders the layout into the given area, sacrificing the bottom row for a `… +N rows` hint
/// when the configured height doesn't fit and the viewport had to be clamped to the terminal.
#[allow(clippy::too_many_arguments)]
fn draw_frame(
    frame: &mut Frame,
    area: Rect,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
    specs: &HashMap<WidgetId, RenderSpec>,
    registry: &render::Registry,
    theme: &Theme,
    general: &General,
    padding: (u16, u16),
    hidden_rows: u16,
    loading: &HashMap<WidgetId, Shape>,
) {
    // Paint the full area first so the padded band takes the theme bg too — otherwise the
    // terminal bg would show through the padding ring and fight the theme surface.
    paint_viewport_bg(frame, area, theme);
    let content = shrink(area, padding);
    if hidden_rows > 0 && content.height > 1 {
        let layout_area = Rect {
            height: content.height - 1,
            ..content
        };
        let hint_area = Rect {
            y: content.y + content.height - 1,
            height: 1,
            ..content
        };
        layout::draw(
            frame,
            layout_area,
            root,
            payloads,
            specs,
            registry,
            theme,
            general,
            loading,
        );
        render_clip_hint(frame, hint_area, hidden_rows, theme);
    } else {
        layout::draw(
            frame, content, root, payloads, specs, registry, theme, general, loading,
        );
    }
}

/// Shrinks `area` by `padding` on each axis, clamping so padding larger than the half-extent
/// doesn't produce a negative-sized rect.
fn shrink(area: Rect, (x, y): (u16, u16)) -> Rect {
    let dx = x.min(area.width / 2);
    let dy = y.min(area.height / 2);
    Rect {
        x: area.x + dx,
        y: area.y + dy,
        width: area.width.saturating_sub(dx.saturating_mul(2)),
        height: area.height.saturating_sub(dy.saturating_mul(2)),
    }
}

/// Paints `theme.bg` across the whole splash area before any widget draws, so themes that
/// ship a bg (Tokyo Night, Dracula, etc.) get a solid painted surface instead of inheriting
/// the terminal's own background. `Color::Reset` is a no-op — cells stay terminal-default.
fn paint_viewport_bg(frame: &mut Frame, area: Rect, theme: &Theme) {
    if theme.bg == Color::Reset {
        return;
    }
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);
}

fn render_clip_hint(frame: &mut Frame, area: Rect, hidden_rows: u16, theme: &Theme) {
    let text = format!("… +{hidden_rows} rows (terminal too short)");
    let style = Style::default()
        .fg(theme.text_dim)
        .add_modifier(Modifier::ITALIC);
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
    use crate::payload::{Body, TextBlockData, TextData};
    use async_trait::async_trait;
    use ratatui::{Terminal, backend::TestBackend};

    fn text_payload(line: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData { value: line.into() }),
        }
    }

    fn text_block_payload(lines: &[&str]) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::TextBlock(TextBlockData {
                lines: lines.iter().map(|s| (*s).to_string()).collect(),
            }),
        }
    }

    fn single_widget_tree(id: &str) -> LayoutTree {
        LayoutTree::widget(id)
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
        let theme = Theme::default();
        let loading = HashMap::new();
        terminal
            .draw(|f| {
                draw_frame(
                    f,
                    f.area(),
                    &root,
                    &payloads,
                    &specs,
                    &registry,
                    &theme,
                    &General::default(),
                    (0, 0),
                    7,
                    &loading,
                )
            })
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
        let theme = Theme::default();
        let loading = HashMap::new();
        terminal
            .draw(|f| {
                draw_frame(
                    f,
                    f.area(),
                    &root,
                    &payloads,
                    &specs,
                    &registry,
                    &theme,
                    &General::default(),
                    (0, 0),
                    0,
                    &loading,
                )
            })
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
        let theme = Theme::default();
        terminal
            .draw(|f| render_clip_hint(f, f.area(), 42, &theme))
            .unwrap();
        let row = row_text(&terminal.backend().buffer().clone(), 0);
        assert!(row.contains("…"), "marker missing: {row:?}");
        assert!(row.contains("+42"), "count missing: {row:?}");
    }

    #[test]
    fn copy_rows_copies_vertical_slice() {
        let mut src = Buffer::empty(Rect::new(0, 0, 4, 4));
        src.set_string(0, 0, "AAAA", Style::default());
        src.set_string(0, 1, "BBBB", Style::default());
        src.set_string(0, 2, "CCCC", Style::default());
        src.set_string(0, 3, "DDDD", Style::default());
        let mut dst = Buffer::empty(Rect::new(0, 0, 4, 2));
        copy_rows(&src, 1..3, &mut dst);
        assert_eq!(row_text(&dst, 0), "BBBB");
        assert_eq!(row_text(&dst, 1), "CCCC");
    }

    #[test]
    fn copy_rows_honours_destination_origin() {
        let mut src = Buffer::empty(Rect::new(0, 0, 3, 1));
        src.set_string(0, 0, "XYZ", Style::default());
        // Destination offset at (2, 5) to mimic an inline-viewport-style buffer.
        let mut dst = Buffer::empty(Rect::new(2, 5, 3, 1));
        copy_rows(&src, 0..1, &mut dst);
        assert_eq!(dst.cell((2, 5)).unwrap().symbol(), "X");
        assert_eq!(dst.cell((4, 5)).unwrap().symbol(), "Z");
    }

    #[test]
    fn render_full_into_buffer_paints_full_height() {
        let root = single_widget_tree("x");
        let mut payloads = HashMap::new();
        payloads.insert(
            "x".into(),
            text_block_payload(&["r0", "r1", "r2", "r3", "r4", "r5"]),
        );
        let specs = HashMap::new();
        let registry = render::Registry::with_builtins();
        let theme = Theme::default();
        let loading = HashMap::new();
        let buf = render_full_into_buffer(
            20,
            6,
            &root,
            &payloads,
            &specs,
            &registry,
            &theme,
            &General::default(),
            (0, 0),
            &loading,
        );
        assert!(row_text(&buf, 0).starts_with("r0"));
        assert!(row_text(&buf, 5).starts_with("r5"));
    }

    #[test]
    fn finalize_draw_flushes_top_rows_and_reveals_bottom() {
        let root = single_widget_tree("x");
        let mut payloads = HashMap::new();
        payloads.insert(
            "x".into(),
            text_block_payload(&["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7"]),
        );
        let specs = HashMap::new();
        let registry = render::Registry::with_builtins();
        // 10-row terminal, 4-row inline viewport, 8-row requested layout: 4 rows are pushed to
        // scrollback (rows r0..r3), bottom 4 rows (r4..r7) land in the viewport. No clip hint.
        let backend = TestBackend::new(20, 10);
        let mut terminal = make_terminal(backend, 4).unwrap();
        let theme = Theme::default();
        let loading = HashMap::new();
        finalize_draw(
            &mut terminal,
            &root,
            &payloads,
            &specs,
            &registry,
            &theme,
            &General::default(),
            (0, 0),
            8,
            4,
            &loading,
        )
        .unwrap();
        let buf = terminal.backend().buffer().clone();
        let rows: Vec<String> = (0..buf.area.height)
            .map(|y| row_text(&buf, y).trim_end().to_string())
            .collect();
        let combined = rows.join("\n");
        assert!(
            rows.iter().any(|r| r.contains("r7")),
            "bottom row r7 missing: {rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.contains("r0")),
            "scrollback row r0 missing: {rows:?}"
        );
        assert!(
            !combined.contains("…"),
            "clip hint should be suppressed after flush: {combined:?}"
        );
    }

    #[test]
    fn finalize_draw_without_clipping_falls_through_to_draw_final() {
        let root = single_widget_tree("x");
        let mut payloads = HashMap::new();
        payloads.insert("x".into(), text_payload("fits"));
        let specs = HashMap::new();
        let registry = render::Registry::with_builtins();
        let backend = TestBackend::new(20, 10);
        let mut terminal = make_terminal(backend, 3).unwrap();
        let theme = Theme::default();
        let loading = HashMap::new();
        finalize_draw(
            &mut terminal,
            &root,
            &payloads,
            &specs,
            &registry,
            &theme,
            &General::default(),
            (0, 0),
            3,
            3,
            &loading,
        )
        .unwrap();
        let buf = terminal.backend().buffer().clone();
        let joined: String = (0..buf.area.height).map(|y| row_text(&buf, y)).collect();
        assert!(joined.contains("fits"));
        assert!(!joined.contains("…"));
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
        assert!(should_refresh(&cached, &widget("a", "basic_static")));
    }

    #[test]
    fn should_refresh_when_entry_is_stale() {
        let mut cached = HashMap::new();
        cached.insert(
            "a".into(),
            CacheEntry {
                refreshed_at: 0,
                ttl_seconds: 60,
                kind: CacheEntryKind::Ok,
                payload: text_payload("x"),
            },
        );
        assert!(should_refresh(&cached, &widget("a", "basic_static")));
    }

    #[test]
    fn should_skip_when_entry_is_fresh() {
        let mut cached = HashMap::new();
        cached.insert("a".into(), CacheEntry::new(text_payload("x"), 60));
        assert!(!should_refresh(&cached, &widget("a", "basic_static")));
    }

    fn static_widget(id: &str, text: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: "basic_static".into(),
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
            &General::default(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        assert!(fresh.contains_key("greeting"));
        let key = registry
            .get_cached("basic_static")
            .unwrap()
            .cache_key(&fetch_context(
                &widgets[0],
                &General::default(),
                None,
                Duration::from_secs(0),
            ));
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
            &General::default(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        let entries = load_entries(
            Some(&cache),
            &registry,
            &widgets[1..],
            &General::default(),
            &HashMap::new(),
        );
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
            &General::default(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        let entries = load_entries(
            Some(&cache),
            &registry,
            &second,
            &General::default(),
            &HashMap::new(),
        );
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
            fetcher: "basic_static".into(),
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
            &General::default(),
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
            .get_cached("basic_static")
            .unwrap()
            .cache_key(&fetch_context(
                &widgets[0],
                &General::default(),
                None,
                Duration::from_secs(0),
            ));
        let _held = cache.try_lock(&key).expect("first acquire");

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
            &HashMap::new(),
            &General::default(),
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
            &General::default(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;
        assert!(fresh.is_empty());
    }

    /// Returning `Err` from a fetcher must surface a placeholder payload *and* write the failure
    /// to the cache (with `CacheEntryKind::Err` and a short TTL) so the next render sees the `⚠`
    /// instead of stale success data. Regression for #95 — pre-fix this path silently dropped the
    /// error via `.ok()?.ok()?`.
    #[tokio::test]
    async fn fetch_all_lifts_err_to_placeholder_and_cache() {
        struct Boom;

        #[async_trait]
        impl fetcher::Fetcher for Boom {
            fn name(&self) -> &str {
                "boom"
            }
            fn safety(&self) -> fetcher::Safety {
                fetcher::Safety::Safe
            }
            fn description(&self) -> &'static str {
                "test fixture"
            }
            fn shapes(&self) -> &[Shape] {
                &[Shape::Text]
            }
            async fn fetch(
                &self,
                _: &fetcher::FetchContext,
            ) -> Result<Payload, fetcher::FetchError> {
                Err(fetcher::FetchError::Failed("boom".into()))
            }
        }

        let mut registry = Registry::default();
        registry.register(std::sync::Arc::new(Boom));
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let widgets = vec![widget("x", "boom")];

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
            &HashMap::new(),
            &General::default(),
            &HashMap::new(),
            Duration::from_secs(1),
        )
        .await;

        let payload = fresh.get("x").expect("error must surface as a payload");
        match &payload.body {
            Body::Error(err) => {
                assert_eq!(err.kind, crate::payload::ErrorKind::Fetch);
                assert!(
                    err.message.contains("boom"),
                    "error message must appear in the placeholder: {:?}",
                    err.message
                );
            }
            other => panic!("expected Body::Error error placeholder, got {other:?}"),
        }

        let key = registry
            .get_cached("boom")
            .unwrap()
            .cache_key(&fetch_context(
                &widgets[0],
                &General::default(),
                None,
                Duration::from_secs(0),
            ));
        let stored = cache.load(&key).expect("error entry must be cached");
        assert_eq!(stored.kind, CacheEntryKind::Err);
        assert_eq!(stored.ttl_seconds, ERROR_CACHE_TTL_SECS);
    }

    /// Deadline expiry turns into a distinct `CacheEntryKind::Timeout` placeholder so diagnostics
    /// can tell "slow" from "broken".
    #[tokio::test]
    async fn fetch_all_lifts_timeout_to_placeholder_and_cache() {
        struct Slow;

        #[async_trait]
        impl fetcher::Fetcher for Slow {
            fn name(&self) -> &str {
                "slow"
            }
            fn safety(&self) -> fetcher::Safety {
                fetcher::Safety::Safe
            }
            fn description(&self) -> &'static str {
                "test fixture"
            }
            fn shapes(&self) -> &[Shape] {
                &[Shape::Text]
            }
            async fn fetch(
                &self,
                _: &fetcher::FetchContext,
            ) -> Result<Payload, fetcher::FetchError> {
                tokio::time::sleep(Duration::from_secs(30)).await;
                unreachable!("deadline must fire before the sleep resolves")
            }
        }

        let mut registry = Registry::default();
        registry.register(std::sync::Arc::new(Slow));
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path().to_path_buf()).unwrap();
        let widgets = vec![widget("x", "slow")];

        let fresh = fetch_all(
            &registry,
            Some(&cache),
            &widgets,
            &HashMap::new(),
            &General::default(),
            &HashMap::new(),
            Duration::from_millis(10),
        )
        .await;

        let payload = fresh.get("x").expect("timeout must surface as a payload");
        match &payload.body {
            Body::Error(err) => {
                assert_eq!(err.kind, crate::payload::ErrorKind::Timeout);
                assert!(
                    err.message.contains("timed out"),
                    "timeout message must appear: {:?}",
                    err.message
                );
            }
            other => panic!("expected Body::Error timeout placeholder, got {other:?}"),
        }

        let key = registry
            .get_cached("slow")
            .unwrap()
            .cache_key(&fetch_context(
                &widgets[0],
                &General::default(),
                None,
                Duration::from_secs(0),
            ));
        let stored = cache.load(&key).expect("timeout entry must be cached");
        assert_eq!(stored.kind, CacheEntryKind::Timeout);
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
    fn derive_shape_uses_renderer_single_accept_when_present() {
        // Renderers are single-shape: `text_plain` accepts only `Text`. Pairing it with
        // `github_repo` (shapes = [Entries, TextBlock, Text]) must select `Text`, the only
        // intersection — even though `Entries` / `TextBlock` come first in the fetcher's
        // own preference order.
        let registry = Registry::with_builtins();
        let render_registry = render::Registry::with_builtins();
        let w = widget_with_render("s", "github_repo", Some("text_plain"));
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
    fn derive_shape_uses_renderer_first_shape_when_fetcher_unknown() {
        let registry = Registry::with_builtins();
        let render_registry = render::Registry::with_builtins();
        let w = widget_with_render("t", "definitely_not_a_fetcher", Some("grid_calendar"));
        assert_eq!(
            derive_shape(&w, &registry, &render_registry),
            Some(Shape::Calendar)
        );
    }

    #[test]
    fn partition_by_shape_support_keeps_widget_with_compatible_renderer() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget_with_render(
            "d",
            "system_disk_usage",
            Some("gauge_circle"),
        )];
        let shapes = single_shape("d", Shape::Ratio);
        let (valid, invalid) = partition_by_shape_support(&widgets, &shapes, &registry);
        assert_eq!(valid.len(), 1);
        assert!(invalid.is_empty());
    }

    #[test]
    fn partition_by_shape_support_passes_through_when_shape_is_missing() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget_with_render("clock", "clock", Some("text_plain"))];
        let (valid, invalid) = partition_by_shape_support(&widgets, &HashMap::new(), &registry);
        assert_eq!(valid.len(), 1);
        assert!(invalid.is_empty());
    }

    #[test]
    fn partition_by_shape_support_rejects_incompatible_renderer() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget_with_render(
            "d",
            "system_disk_usage",
            Some("grid_calendar"),
        )];
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
        let widgets = vec![widget_with_render(
            "x",
            "no_such_fetcher",
            Some("grid_calendar"),
        )];
        let shapes = single_shape("x", Shape::Calendar);
        let (valid, invalid) = partition_by_shape_support(&widgets, &shapes, &registry);
        assert_eq!(valid.len(), 1);
        assert!(invalid.is_empty());
    }

    #[test]
    fn partition_by_known_fetcher_separates_unknown_names() {
        let registry = Registry::with_builtins();
        let widgets = vec![
            widget("ok", "clock"),
            widget("typo", "no_such_fetcher"),
            widget("static", "basic_static"),
        ];
        let (known, unknown) = partition_by_known_fetcher(&widgets, &registry);
        let known_ids: Vec<_> = known.iter().map(|w| w.id.clone()).collect();
        let unknown_ids: Vec<_> = unknown.iter().map(|w| w.id.clone()).collect();
        assert_eq!(known_ids, vec!["ok".to_string(), "static".to_string()]);
        assert_eq!(unknown_ids, vec!["typo".to_string()]);
    }

    #[test]
    fn refresh_payloads_surfaces_unknown_fetcher_placeholder() {
        // Regression for #121: a typo'd or stale fetcher name used to drop the widget silently.
        // After the fix, the widget keeps its slot and shows `⚠ unknown fetcher: <name>`.
        let registry = Registry::with_builtins();
        let unknown_widgets = vec![widget("typo", "no_such_fetcher")];
        let buckets = WidgetBuckets {
            cached: &[],
            gated: &[],
            unknown: &unknown_widgets,
            shape_invalid: &[],
        };
        let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
        let _ = refresh_payloads(
            &None,
            &registry,
            &buckets,
            &General::default(),
            &HashMap::new(),
            &mut payloads,
        );
        let payload = payloads.get("typo").expect("unknown fetcher must surface");
        match &payload.body {
            Body::Error(err) => {
                assert_eq!(err.kind, crate::payload::ErrorKind::UnknownFetcher);
                assert!(
                    err.message.contains("no_such_fetcher"),
                    "placeholder must mention the unknown fetcher name: {:?}",
                    err.message
                );
            }
            other => panic!("expected Body::Error placeholder, got {other:?}"),
        }
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

    #[test]
    fn split_by_fetcher_kind_and_compute_realtime_payloads_use_only_realtime_widgets() {
        let registry = Registry::with_builtins();
        let widgets = vec![widget("clock", "clock"), static_widget("static", "cached")];
        let (cached, realtime) = split_by_fetcher_kind(&widgets, &registry);
        assert_eq!(
            cached.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["static"]
        );
        assert_eq!(
            realtime.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["clock"]
        );

        let payloads = compute_realtime_payloads(
            &registry,
            &widgets,
            &General::default(),
            &single_shape("clock", Shape::Calendar),
        );
        assert_eq!(payloads.len(), 1);
        assert!(matches!(
            payloads.get("clock").map(|payload| &payload.body),
            Some(Body::Calendar(_))
        ));
    }

    #[test]
    fn load_entries_without_cache_returns_empty() {
        let registry = Registry::with_builtins();
        let widgets = vec![static_widget("static", "cached")];
        let entries = load_entries(
            None,
            &registry,
            &widgets,
            &General::default(),
            &HashMap::new(),
        );
        assert!(entries.is_empty());
    }

    #[test]
    fn render_specs_collect_only_widgets_with_explicit_renderers() {
        let widgets = vec![
            widget_with_render("plain", "clock", None),
            widget_with_render("fancy", "clock", Some("text_plain")),
        ];
        let specs = render_specs(&widgets);
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs.get("fancy").map(RenderSpec::renderer_name),
            Some("text_plain")
        );
        assert!(!specs.contains_key("plain"));
    }

    #[test]
    fn animation_still_due_only_while_something_animates() {
        // Regression: minimal presets on a cold cache used to burn the full 2s animation window
        // because loading spinners flipped `animated = true`. Once the daemon clears the loading
        // set, nothing needs to animate and the window stops being binding even though the
        // deadline hasn't expired yet.
        assert!(
            animation_still_due(true, true, false),
            "real animation pending → still due even with no loading"
        );
        assert!(
            animation_still_due(true, false, true),
            "loading spinners pending → still due"
        );
        assert!(
            !animation_still_due(true, false, false),
            "deadline not reached but nothing to animate → not due"
        );
        assert!(
            !animation_still_due(false, true, true),
            "deadline reached → animation no longer due regardless"
        );
    }

    #[test]
    fn loop_should_exit_requires_both_axes_to_settle() {
        // The load axis (daemon) and the render axis (animation) are independent — regression
        // for the "wait returned as soon as daemon exited, cutting off animation" bug.
        assert!(
            loop_should_exit(true, false, false),
            "daemon done, no animation → exit"
        );
        assert!(
            !loop_should_exit(true, true, false),
            "animation still playing → keep ticking"
        );
        assert!(
            !loop_should_exit(false, false, false),
            "daemon still running → wait"
        );
        assert!(!loop_should_exit(false, true, false), "both pending → wait");
        assert!(
            loop_should_exit(false, true, true),
            "hard deadline always wins"
        );
        assert!(
            loop_should_exit(true, true, true),
            "hard deadline always wins"
        );
    }

    #[test]
    fn compute_loading_surfaces_cached_widgets_without_payload() {
        let cached = vec![widget("pending", "github_my_prs"), widget("ready", "clock")];
        let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
        payloads.insert("ready".into(), text_payload("14:00"));
        let mut shapes: HashMap<WidgetId, Shape> = HashMap::new();
        shapes.insert("pending".into(), Shape::Entries);
        shapes.insert("ready".into(), Shape::Text);
        let entries = HashMap::new();
        let loading = compute_loading(&cached, &payloads, &entries, &shapes, false);
        assert_eq!(loading.len(), 1);
        assert_eq!(loading.get("pending"), Some(&Shape::Entries));
        assert!(!loading.contains_key("ready"));
    }

    #[test]
    fn compute_loading_flags_stale_entry_only_under_wait() {
        // Regression for: under `--wait` a stale cache entry still shows last-run data instead
        // of telegraphing that a refetch is in flight.
        let cached = vec![widget("stale", "github_my_prs")];
        let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
        payloads.insert("stale".into(), text_payload("last-run"));
        let mut shapes: HashMap<WidgetId, Shape> = HashMap::new();
        shapes.insert("stale".into(), Shape::Entries);
        let mut entries = HashMap::new();
        entries.insert(
            "stale".into(),
            CacheEntry {
                refreshed_at: 0,
                ttl_seconds: 60,
                kind: CacheEntryKind::Ok,
                payload: text_payload("last-run"),
            },
        );

        // Without --wait: stale cache is shown as-is, no spinner.
        let without_wait = compute_loading(&cached, &payloads, &entries, &shapes, false);
        assert!(
            without_wait.is_empty(),
            "stale cache should render without spinner off the wait path"
        );

        // With --wait: stale cache → spinner because the daemon is on the critical path.
        let with_wait = compute_loading(&cached, &payloads, &entries, &shapes, true);
        assert_eq!(with_wait.get("stale"), Some(&Shape::Entries));
    }

    #[test]
    fn compute_loading_keeps_fresh_entry_off_spinner_even_under_wait() {
        let cached = vec![widget("fresh", "github_my_prs")];
        let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
        payloads.insert("fresh".into(), text_payload("now"));
        let mut shapes: HashMap<WidgetId, Shape> = HashMap::new();
        shapes.insert("fresh".into(), Shape::Entries);
        let mut entries = HashMap::new();
        entries.insert("fresh".into(), CacheEntry::new(text_payload("now"), 600));
        let loading = compute_loading(&cached, &payloads, &entries, &shapes, true);
        assert!(loading.is_empty());
    }

    #[test]
    fn has_loading_widget_true_when_any_cached_widget_lacks_payload() {
        let cached = vec![widget("a", "github_my_prs"), widget("b", "clock")];
        let mut payloads: HashMap<WidgetId, Payload> = HashMap::new();
        payloads.insert("a".into(), text_payload("x"));
        let entries = HashMap::new();
        assert!(has_loading_widget(&cached, &payloads, &entries, false));
        payloads.insert("b".into(), text_payload("y"));
        assert!(!has_loading_widget(&cached, &payloads, &entries, false));
    }
}
