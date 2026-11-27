# Changelog

All notable changes to this package will be documented in this file.
## 0.1.1 (2026-11-27)

### Features

#### Guided Antigravity installation and Google sign-in

Guide first-time Antigravity users through a TUI setup flow before starting a
conversation. Offer a verified, per-user managed runtime download when the CLI
is missing, show install progress, hand off to the official Google sign-in flow,
and verify account access before continuing. Noninteractive entry points never
install software or begin sign-in automatically.

Launch full-access Antigravity sessions with the official
`--dangerously-skip-permissions` option while preserving approval-enabled and
Plan modes.

#### Use Google Antigravity subscriptions through the official ACP runtime

Add `roder --backend antigravity` and subscription sign-in via the official
`agy acp` CLI. Discover models from the account, stream agent activity, and route
tool permissions through Roder without extracting tokens or falling back to
Gemini API-key billing. Make shared backend provider and approval labels work
with runtimes other than Codex.
