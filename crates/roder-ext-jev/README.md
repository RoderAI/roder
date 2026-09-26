# roder-ext-jev

`roder-ext-jev` is the goal-directed browser tool for [Roder](https://roder.sh).

## What It Does

Exposes a `jev_browse` tool that runs a bounded browser task against Chrome over
the DevTools Protocol: given a starting URL and an observable goal, it observes
the page, chooses one operation at a time, executes it, and returns the action
trace with the observed final page.

The implementation is a Rust port of
[Jev Ultrafast](https://github.com/browser-use/jev-ultrafast) (MIT), pinned to
revision `1231850a`. The two scripts that must run inside the page are vendored
verbatim; the agent loop, indexed action space, decision contract and text
helper are ported, and fixtures recorded from the upstream Python pin the port's
behaviour.

## Behaviour

- Roder reuses a running Chrome DevTools endpoint or starts a visible Chrome on
  its own profile.
- Tasks run in a foreground tab by default, leaving the final page on screen.
- Typing is served by Roder's own configured chat-completions provider.
- Page content is untrusted: a `done` result is an agent claim to be checked
  against the observed page.

Requires `JEV_API_KEY`. In Roder's default policy mode each goal needs approval
before the browser starts; plan mode denies it.

See [`docs/jev-browser.md`](https://github.com/RoderAI/roder/blob/master/docs/jev-browser.md)
for configuration and known limitations.
