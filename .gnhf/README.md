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
├── run.sh                 # launches one loop (foreground) on a gnhf/ branch
└── runs/                  # gnhf runtime state (git-ignored)
```

## Usage

Start fresh on `main` with a clean tree, then pick a loop:

```sh
git checkout main && git pull --ff-only

./.gnhf/run.sh coverage   # tests-only loop
./.gnhf/run.sh perf       # perf-only loop
```

`run.sh` runs in the foreground so you can watch the live gnhf TUI and
Ctrl+C when you're done. Override the token cap with `GNHF_MAX_TOKENS`:

```sh
GNHF_MAX_TOKENS=10000000 ./.gnhf/run.sh coverage
```

When the loop ends (token cap, runtime cap, or Ctrl+C), the commits live on
the `gnhf/...` branch gnhf created. Cherry-pick the good ones onto a feature
branch, open a PR, merge with `--merge`.

Only one loop at a time per checkout — `gnhf` writes to a single `gnhf/`
branch in this repo. Run separate loops in separate clones if you want them
in parallel.

## Adding a new loop

1. Drop a new `<name>-loop.md` into `prompts/`. Write a continuous objective
   with a measurable metric and an "exit non-zero if metric did not move"
   gate, so failed iterations roll back automatically.
2. `./.gnhf/run.sh <name>` picks it up automatically.
