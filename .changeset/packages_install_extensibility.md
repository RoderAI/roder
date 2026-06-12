---
roder-api: minor
roder-app-server: minor
roder-cli: minor
roder-commands: minor
roder-config: minor
roder-ext-process-host: minor
roder-protocol: minor
roder-sdk-typescript: minor
roder-sdk-python: minor
roder-tui: minor
---

# One-command Roder package install (`roder install npm:/git:/path`)

Roder packages bundle process extensions, skills, slash commands, and themes
behind a root `roder.toml` manifest. Install from npm, git (shorthand, SSH,
raw URLs, pinned refs), or local paths; manage with `roder packages
list|resources|enable|disable|approve|filter|sync|init`, `roder remove`,
`roder update`, and ephemeral `-e` loading. Resources surface through the
existing skills/commands/theme registries; the process-extension protocol
gains manifest-declared tool providers served over `tools/call`. New
app-server `packages/*` methods, a `/packages` builtin, and a Packages
palette section round out the surfaces. npm lifecycle scripts stay disabled
unless `--allow-scripts` is passed, and package process extensions never
launch before explicit approval.
