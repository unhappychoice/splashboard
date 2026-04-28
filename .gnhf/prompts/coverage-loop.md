You are working on splashboard. Read AGENTS.md and CLAUDE.md before doing anything.

Continuous objective (no end state):
Push splashboard's line + branch coverage upward, forever, one commit at a time.
Every iteration measures current coverage, finds the highest-leverage uncovered
region, adds tests, and commits. The number only goes up.

Procedure for THIS iteration:
1. Measure baseline:
     cargo install cargo-llvm-cov --locked   # if missing
     cargo llvm-cov --workspace --summary-only
   Record the overall line%.

2. Read notes.md if it exists. Skip targets already attempted in prior iterations.

3. Pick ONE target by priority:
   a. File with < 60% line coverage that has real logic.
   b. Function with high cyclomatic complexity and 0 test hits.
   c. Error/fallback branch never exercised (Err(_), None =>, _ =>, unwrap_or, ok()?).
   d. Realtime/cached fetcher path with no test.
   e. Renderer's render() for a Shape variant not covered via render/test_utils.rs.
   Skip generated code, main.rs arg parsing, trivial code.

4. Add tests, NOT new production code:
   - #[cfg(test)] mod tests next to the code.
   - Renderer tests use src/render/test_utils.rs.
   - Tests touching SPLASHBOARD_HOME or process env MUST take paths::TEST_ENV_LOCK.
   - Assert on observable output (rendered cells, returned Body, error variants).
   - If genuinely untestable as written, pick a different target. Do NOT refactor
     production code in a coverage iteration.

5. Re-measure. Overall line% MUST be strictly higher than step 1. If not, exit
   non-zero so gnhf rolls back.

6. Quality gate (all must pass, else exit non-zero):
   - cargo test
   - cargo clippy --all-targets -- -D warnings
   - cargo fmt --all -- --check

7. Commit:
   - The structured `summary` field and the commit body MUST be written in English. No Japanese, no other languages.
   - Conventional: test(scope): cover <area>
   - Body includes: before% -> after% (+Xpp), which file/function, why it was the pick.
   - Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
   - Append one line to notes.md: "<commit-sha-short> covered <file>::<fn> (X.X% -> Y.Y%)"

Hard rules:
- Tests-only commit. No production code changes (renaming a private item to make
  it testable is OK, keep diff minimal).
- No new top-level deps. cargo-llvm-cov is dev-only.
- No #[ignore], no --no-verify, no skipping flaky tests.
- Do NOT reintroduce closed designs (#5 plugin protocol, #20 command widget,
  XDG paths, screensaver mode).

Coverage only goes up.
