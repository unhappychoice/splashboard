You are working on splashboard. Read AGENTS.md and CLAUDE.md before doing anything.

Continuous objective (no end state):
Drive cold-start render latency downward, forever, one commit at a time. The
killer feature is fast cached first-paint on shell startup and cd. Every
millisecond shaved is felt on every prompt. The number only goes down.

Procedure for THIS iteration:
1. Measure baseline. Release build, warm pagecache, hyperfine 20 runs, take median:
     cargo build --release
     ./target/release/splashboard render --once >/dev/null 2>&1 || true
     hyperfine --warmup 3 --runs 20 './target/release/splashboard render --once'
   Record median + stddev.

2. Read notes.md if it exists. Skip targets already attempted.

3. Pick ONE target by priority:
   a. Sync I/O on the cold path that can become lazy/cached/skipped on first paint
      (cache-miss falls through to background refresh — that pattern exists, extend it).
   b. serde deserialize of a large blob that can be bounded/streamed/cached parsed.
   c. Panic-on-startup safety check that can be relaxed to a warning.
   d. RealtimeFetcher::compute that is NOT actually < 1ms — fix or demote to cached.
   e. Renderer doing avoidable work for empty bodies (empty-state short-circuit
      lives in render::render_payload — make sure nobody bypasses it).
   f. Crate features / build profile tweaks that reduce binary size or startup
      symbol resolution without behavior change.
   Skip anything that changes user-visible output. Skip correctness-for-speed trades.

4. Implement. Small, surgical diff. Preserve Shape × Fetcher × Renderer separation.
   No new top-level deps unless removing a heavier one in the same commit.
   No unsafe to chase microseconds.

5. Re-measure with the same hyperfine command. New median MUST be lower than
   step 1 by MORE than hyperfine's stddev. If not, the change is noise — exit
   non-zero so gnhf rolls back.

6. Quality gate (all must pass, else exit non-zero):
   - cargo test
   - cargo clippy --all-targets -- -D warnings
   - cargo fmt --all -- --check
   - Visual smoke: ./target/release/splashboard render --once still produces the
     expected splash for the default config.

7. Commit:
   - Conventional: perf(scope): <what was sped up>
   - Body includes: before ms ± stddev -> after ms ± stddev (delta + %), what was
     slow, why, what changed, machine info (uname -a; rustc --version).
   - Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
   - Append one line to notes.md: "<commit-sha-short> sped up <area> (Xms -> Yms)"

Hard rules:
- Output of render --once must be byte-identical for the default config (or
  visually identical if timestamps/randomness — document any intentional diff).
- No reintroducing closed designs (#5, #20, XDG paths, screensaver mode).
- "Looks faster" doesn't count. If you can't measure a real improvement, exit non-zero.

Latency only goes down.
