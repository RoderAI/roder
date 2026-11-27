# Antigravity agent backend

Subscription-backed Google agents for [Roder](https://roder.sh).

Runs the official `agy acp` stdio server using your Google Antigravity
subscription. Install the official Antigravity CLI, then run
`roder auth login antigravity` in a terminal.
Roder uses the CLI's default profile with consistent file-backed OAuth storage
for login and backend processes, and does not read or copy credentials.

API-key and cloud-auth selector environment variables are removed from the
child process so they cannot override subscription authentication. Normal
connections do not launch a browser; interactive authentication is a separate,
explicit command.

Models and thinking levels come from ACP session capabilities, not a static
catalogue. Session resume requires the server's advertised load capability.
The agent owns tools; requests for unsupported client filesystem or terminal
services are rejected. Plan policy is rejected if the agent does not advertise
a plan mode. Approval choices use the exact option IDs supplied by the agent.

Run deterministic protocol tests with
`cargo test -p roder-ext-antigravity-backend`.
After logging in, explicitly opt into a live subscription inference:
`RODER_LIVE_ANTIGRAVITY=1 cargo test -p roder-ext-antigravity-backend --test live -- --ignored --nocapture`.
