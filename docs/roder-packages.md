# Roder Packages

Roder packages bundle process extensions, skills, slash commands, and themes
into one installable unit, fetched from npm, git, or a local path with a
single command. Authors publish a `roder.toml` manifest at the root of their
repository; users install the repository and the bundled resources flow into
Roder's existing registries.

```sh
roder install npm:@foo/roder-tools
roder install npm:@foo/roder-tools@1.2.0      # pinned version
roder install git:github.com/user/repo
roder install git:github.com/user/repo@v1     # pinned tag or commit
roder install https://github.com/user/repo    # raw URLs work too
roder install ./relative/path/to/package
roder install /absolute/path/to/package

roder packages list
roder packages resources <package-id>
roder remove <spec-or-id>
roder update                                  # update all (skips pinned)
roder update <spec-or-id>                     # update one (even pinned)
```

> **Security:** unlike full-access harnesses, Roder gates everything that
> executes code. npm lifecycle scripts never run unless you pass
> `--allow-scripts`. Process extensions never launch until you run
> `roder packages approve <id>`. Approved extensions still execute with your
> local permissions, so review the source of third-party packages first.

## Spec grammar

| Spec | Meaning |
| --- | --- |
| `npm:pkg`, `npm:@scope/pkg`, `npm:@scope/pkg@1.2.3` | npm registry package (versioned specs are pinned) |
| `git:github.com/user/repo[@ref]` | shorthand, expands to `https://github.com/user/repo` |
| `git:git@github.com:user/repo[@ref]` | scp-style SSH |
| `https://`, `http://`, `ssh://`, `git://` URLs (`git:` prefix optional) | protocol git URLs, optional trailing `@ref` |
| `git:file:///path/to/repo[@ref]` | local git repository (useful for testing) |
| `/abs/path`, `./rel/path`, `~/path` | local directory, loaded in place (never copied or deleted) |

Identity is the npm name, the git URL without ref, or the resolved local
path. Reinstalling the same identity updates the record in place.

## Scopes, stores, and settings

| Scope | Settings file | Install store |
| --- | --- | --- |
| User (default) | `~/.roder/packages.json` | `~/.roder/packages/{npm,git}/...` |
| Project (`-l`) | `<workspace>/.roder/packages.json` | `<workspace>/.roder/packages/...` |

Project entries win when the same identity exists in both scopes (the user
entry is listed as shadowed). `.roder/packages.json` can be committed and
shared with a team; teammates run `roder packages sync` to materialize any
missing project packages. There is no silent auto-install: syncing is an
explicit action because Roder has no workspace trust gate yet.

Pin npm operations to a wrapper (mise, asdf) in `~/.roder/config.toml`:

```toml
[packages]
npm_command = ["mise", "exec", "node@22", "--", "npm"]
```

## Update semantics

- `roder update` updates every non-pinned package and reconciles pinned git
  clones back to their configured ref (fetch, checkout, clean).
- Pinned npm specs (`npm:pkg@1.2.3`) are skipped by bulk update.
- `roder update <spec-or-id>` updates one package even when pinned.
- Binary self-update is not part of this command; distribution tooling owns
  that.

## What a package provides

One package can ship any mix of four resource kinds:

| Kind | Activates | Where it lands |
| --- | --- | --- |
| Skills | on install | skills registry (`roder skills list`), source `package://<id>/...` |
| Commands | on install | slash-command registry as `/name`; user and workspace commands shadow package commands; two packages shipping the same name is an error |
| Themes | on install | theme discovery; user and project themes win basename collisions |
| Process extensions | **after `roder packages approve <id>`** | the process-extension host, like a `[[process_extensions]]` config entry |

Per-resource control: `roder packages disable <pkg>:<kind>/<name>`,
`roder packages enable ...`, or disable the whole package with
`roder packages disable <pkg>`. Resource ids are shown by
`roder packages resources <pkg>`.

## Authoring a package

Scaffold one:

```sh
roder packages init my-pack
roder install ./my-pack
```

### Manifest: `roder.toml` at the repository root

```toml
[package]
id = "my-pack"            # required: lowercase alnum plus - _ .
name = "My Pack"          # optional display name
version = "0.1.0"         # optional
description = "..."       # optional

[resources]                # optional; omit to use conventional directories
extensions = ["extensions/wordtools/roder-extension.toml"]
skills = ["skills"]        # directories or globs, package-root-relative
commands = ["commands"]
themes = ["themes"]
```

Manifest precedence at a package root:

1. `roder.toml` (works in plain git repos, no Node required)
2. `package.json` with a `"roder"` key (same arrays; add the `roder-package`
   npm keyword for discoverability)
3. Conventional directories: `skills/`, `commands/`, `themes/`, and
   `extensions/*/roder-extension.toml`

A root with none of these still installs but provides zero resources.

### Resource formats

- **Skills**: directories containing `SKILL.md` (Agent Skills format with
  YAML frontmatter), discovered recursively under each declared root.
- **Commands**: `.md` files with YAML frontmatter (`description`,
  `argument-hint`, ...) and a body template — the same format as
  `~/.roder/commands/`.
- **Themes**: `.css` files using the Roder theme subset
  (see `docs/roder-tui-theming.md`).
- **Extensions**: a `roder-extension.toml` process-extension manifest with a
  `[launch]` section (`command`, `args`, optional `cwd`, `env`,
  `startup_timeout_ms`, `event_filter_kinds`). Relative paths resolve against
  the manifest's directory. Extensions declare provided services — inference
  engines, event sinks, and tool providers with statically declared tool
  schemas — and speak newline-delimited JSON-RPC over stdio (protocol 0.2.0,
  see `docs/roder-process-extensions.md`).

### Filters

Settings can narrow what a package loads without forking it
(`packages/set_filters` over the app-server, or edit `packages.json`):

```json
{
  "filters": {
    "commands": ["commands/*.md", "!commands/legacy.md"],
    "skills": [],
    "themes": ["+themes/neon-dusk.css"]
  }
}
```

Omit a key to load all of that kind, `[]` loads none, `!glob` excludes,
`+path`/`-path` force-include/exclude exact paths. Filters narrow the
manifest; they never widen it.

## Ephemeral try-out

Load a package for one run without installing it:

```sh
roder -e ./path/to/package
roder -e npm:@foo/roder-tools          # fetched to a temp dir
roder -e ./pkg --approve-ephemeral-extensions   # also let its extensions launch
```

Nothing is written to `packages.json`; the package disappears after the run.

## Surfaces

- CLI: `roder install/remove/update` and `roder packages
  list|resources|enable|disable|approve|revoke|sync|init`.
- Composer: `/packages ...` mirrors the CLI verbs.
- Ctrl+P palette: a Packages section lists installed packages with
  enable/disable toggles, pending-approval actions, and install/update/sync
  rows.
- App-server JSON-RPC: `packages/list`, `packages/install`,
  `packages/remove`, `packages/update`, `packages/sync`,
  `packages/set_enabled`, `packages/approve_extensions`,
  `packages/set_filters` (see `docs/app-server/api.md`).

## Example

A complete example package lives at `examples/packages/demo-roder-package/`
(skill + command + theme + Python tool extension). Install it from a checkout:

```sh
roder install ./examples/packages/demo-roder-package
roder packages approve demo-roder-package
```

## Testing and offline behavior

Normal tests run offline against fixture packages, a stub npm command, and
local git repositories. Live npm/git installs are exercised only when
`RODER_PACKAGES_LIVE=1` is set. Marketplace catalogs (phase 43) remain a
separate discovery surface; this package layer is how source-spec installs
materialize on disk.
