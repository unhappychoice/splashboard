# .gnhf/

Overnight [gnhf](https://github.com/kunchenguid/gnhf) loop prompts and launcher.

`gnhf` keeps an agent running while you sleep — each iteration makes one
small, committed change toward an objective. These prompts define
**continuous-improvement objectives** (no terminal state): every iteration
moves a metric in one direction.

## Layout

```
.gnhf/
├── prompts/
│   ├── coverage-loop.md   # tests-only loop: line coverage only goes up
│   └── perf-loop.md       # perf-only loop: cold-start latency only goes down
├── run.sh                 # launches both loops in parallel worktrees
└── runs/                  # gnhf runtime state (git-ignored)
```

## Usage

Start fresh on `main` with a clean tree:

```sh
git checkout main && git pull --ff-only
./.gnhf/run.sh
```

Two `gnhf` processes detach into the background, each in its own git worktree
under `../splashboard-gnhf-worktrees/`. Logs land in `/tmp/gnhf-*.log`.

In the morning:

```sh
ls ../splashboard-gnhf-worktrees/
tail /tmp/gnhf-coverage.log /tmp/gnhf-perf.log
```

Review the commit stack on each worktree's `gnhf/...` branch, cherry-pick the
good ones onto a feature branch, open PRs, merge with `--merge`.

## Adding a new loop

1. Drop a new `<name>-loop.md` into `prompts/`. Write a continuous objective
   with a measurable metric and an "exit non-zero if metric did not move"
   gate, so failed iterations roll back automatically.
2. Add a launch line to `run.sh`.
