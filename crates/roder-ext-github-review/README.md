# roder-ext-github-review

`roder-ext-github-review` is the GitHub review publisher integration for [Roder](https://roder.sh).

## What It Does

It turns the structured findings produced by Roder's `/review` sub-turn into a GitHub pull-request review: an overall summary body plus inline comments anchored to the reviewed diff. Findings that cannot be anchored inside the pull request's diff hunks are moved into the summary body instead, because GitHub rejects an entire review when a single comment points outside the diff.

Two backends sit behind one publisher, selected by `[review.publishers.github].mode`:

- `cli` — shells out to the `gh` CLI (`gh pr view`, `gh api`), reusing whatever credentials `gh auth` already has.
- `http` — talks to `https://api.github.com` directly with a bearer token read from `GITHUB_TOKEN`.
- `auto` (default) — prefers the CLI when `gh` is on `PATH`, otherwise falls back to HTTP.

## How It Fits Into Roder

Roder is an agentic software development system with a Rust CLI/TUI, a JSON-RPC app-server, SDKs, package resources, and first-party runtime extensions. This package is released as part of that workspace so downstream users can depend on the same component boundaries that Roder itself uses.

## Links

- Roder website: https://roder.sh
- Repository: https://github.com/RoderAI/roder

## Publishing

This package is versioned and published with the Roder workspace. Before publishing, run:

```sh
make registry-readmes
python3 scripts/generate-knope-config.py --check
```
