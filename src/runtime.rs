use std::collections::HashMap;
use std::io::{self, Stdout, stdout};
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

const DEFAULT_REFRESH_SECS: u64 = 60;
const FAST_DEADLINE: Duration = Duration::from_millis(1500);
const WAIT_DEADLINE: Duration = Duration::from_secs(5);
const DAEMON_DEADLINE: Duration = Duration::from_secs(30);
const VIEWPORT_LINES: u16 = 16;

pub async fn run(config: &Config, config_path: Option<&Path>, wait: bool) -> io::Result<()> {
    let wait = wait || config.general.wait_for_fresh;
    let layout = config.to_layout();
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();

    let entries = load_entries(cache.as_ref(), &registry, &config.widgets);
    let mut payloads = entries_to_payloads(&entries);

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = make_terminal(backend)?;

    // Cache-first: unless --wait, paint what we have from disk immediately so the shell prompt
    // is never blocked on fetch I/O.
    let drew_cached = !wait && !payloads.is_empty();
    if drew_cached {
        draw(&mut terminal, &layout, &payloads)?;
    }

    match daemon::spawn_fetch_daemon(config_path) {
        Ok(child) => {
            if wait {
                // Block on the daemon (with a hard ceiling) and then redraw with whatever it
                // managed to write before the deadline.
                let _ = wait_for_daemon(child, WAIT_DEADLINE).await;
                let entries = load_entries(cache.as_ref(), &registry, &config.widgets);
                payloads = entries_to_payloads(&entries);
                draw(&mut terminal, &layout, &payloads)?;
            }
            // Default path: the child is detached; it keeps fetching after we exit. Next
            // invocation reads its output from the cache.
        }
        Err(_) => {
            // Failing to spawn the daemon is rare (OOM, exec permission). Fall back to inline
            // fetch so the user still gets fresh data this invocation.
            let deadline = if wait { WAIT_DEADLINE } else { FAST_DEADLINE };
            let fresh = fetch_all(
                &registry,
                cache.as_ref(),
                &config.widgets,
                &entries,
                deadline,
            )
            .await;
            let changed = !fresh.is_empty();
            for (id, payload) in fresh {
                payloads.insert(id, payload);
            }
            if !drew_cached || changed {
                draw(&mut terminal, &layout, &payloads)?;
            }
        }
    }

    println!();
    Ok(())
}

/// Runs fetchers and persists the results. Called by the detached `fetch-only` subcommand so the
/// main invocation can exit without waiting for I/O. Errors from individual fetchers are logged
/// nowhere (daemon has no stdio) and simply leave stale cache in place.
pub async fn fetch_and_persist(config: &Config) {
    let cache = Cache::open_default();
    let registry = Registry::with_builtins();
    let entries = load_entries(cache.as_ref(), &registry, &config.widgets);
    let _ = fetch_all(
        &registry,
        cache.as_ref(),
        &config.widgets,
        &entries,
        DAEMON_DEADLINE,
    )
    .await;
}

async fn wait_for_daemon(mut child: tokio::process::Child, deadline: Duration) -> io::Result<()> {
    match tokio::time::timeout(deadline, child.wait()).await {
        Ok(Ok(_status)) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => {
            let _ = child.start_kill();
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "fetch daemon timed out",
            ))
        }
    }
}

fn fetch_context(w: &WidgetConfig, deadline: Duration) -> FetchContext {
    FetchContext {
        widget_id: w.id.clone(),
        format: w.format.clone(),
        timeout: deadline,
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
            let fetcher = registry.get(&w.fetcher)?;
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
        let Some(fetcher) = registry.get(&w.fetcher) else {
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

fn make_terminal<B: Backend>(backend: B) -> io::Result<Terminal<B>> {
    Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(VIEWPORT_LINES),
        },
    )
}

fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    root: &Layout,
    payloads: &HashMap<WidgetId, Payload>,
) -> io::Result<()> {
    terminal.draw(|frame| layout::draw(frame, frame.area(), root, payloads))?;
    Ok(())
}

#[allow(dead_code)]
fn stdout_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    make_terminal(CrosstermBackend::new(stdout()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Body, TextData};

    fn text_payload(line: &str) -> Payload {
        Payload {
            icon: None,
            status: None,
            format: None,
            body: Body::Text(TextData {
                lines: vec![line.into()],
            }),
        }
    }

    fn widget(id: &str, fetcher: &str) -> WidgetConfig {
        WidgetConfig {
            id: id.into(),
            fetcher: fetcher.into(),
            render: crate::config::RenderType::Text,
            format: None,
            refresh_interval: Some(60),
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
            render: crate::config::RenderType::Text,
            format: Some(text.into()),
            refresh_interval: Some(60),
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
            .get("static")
            .unwrap()
            .cache_key(&fetch_context(&widgets[0], Duration::from_secs(0)));
        let loaded = cache.load(&key).unwrap();
        match loaded.payload.body {
            Body::Text(t) => assert_eq!(t.lines, vec!["Hi!".to_string()]),
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
            render: crate::config::RenderType::Text,
            format: Some("Hi!".into()),
            refresh_interval: Some(60),
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
            .get("static")
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
