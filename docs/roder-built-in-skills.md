# Roder Built-In Skills

Roder treats built-in skills as ordinary skills with a first-party source. They use the same `SkillDescriptor`, `SKILL.md` parser, registry, app-server manager, CLI controls, TUI palette rows, discovery catalog entries, and runtime context injection as workspace, user, plugin, and workflow-import skills.

## Registry And Paths

Built-ins are loaded from repo-authored assets and receive canonical paths shaped like:

```text
roder-builtin://vcs-snapshot/SKILL.md
```

Installed and workspace skills can share a name with a built-in. Name selectors are accepted only when the name resolves to one skill; use the canonical path when a name is ambiguous.

## Exposure

`global` skills appear in the compact global skill index when enabled. Use this for small, generally useful guidance that should be visible before any explicit invocation.

`direct-only` skills stay out of the global index. They are injected only when the user invokes them directly with `$skill-name` or `${skill-name}`, or when a feature binding activates them. The built-in `vcs-snapshot` skill defaults to direct-only so `/snapshot` can use it without increasing every turn's prompt.

## Configuration

Skill config uses the same rule list for built-in and installed skills:

```toml
[[skills.config]]
path = "roder-builtin://vcs-snapshot/SKILL.md"
enabled = false
exposure = "global"
```

Rules may target `name` or `path`. Later matching rules win for fields they set. Disabling a built-in produces a diagnostic on that skill and any required feature binding must refuse clearly.

## App-Server, CLI, And TUI Controls

The app-server exposes:

- `skills/list`
- `skills/read`
- `skills/setEnabled`
- `skills/setExposure`

The CLI mirrors those controls:

```sh
roder skills list
roder skills enable vcs-snapshot
roder skills disable vcs-snapshot
roder skills exposure vcs-snapshot global
roder skills exposure roder-builtin://vcs-snapshot/SKILL.md direct-only
```

The Ctrl+K palette includes a Skills section with rows for reading a skill, toggling enabled state, and switching exposure. The Settings section also includes a Skills manager row that opens the filtered Skills section. Rows show source, activation, exposure, canonical path, and description.

## Feature Bindings

Built-in features should bind to skills instead of embedding large prompt bodies in command or TUI code. `/snapshot` is the reference pattern:

- command metadata declares a required `FeatureSkillBinding`
- expansion resolves the skill through the runtime registry
- direct-only skills are still eligible for feature-bound activation
- disabled required skills stop expansion with a clear diagnostic

This keeps feature guidance editable, discoverable, configurable, and testable through the same path as user skills.

## Discovery And Evals

Enabled and disabled skills appear in the lazy discovery catalog with source, status, exposure, path, tags, and a markdown detail page. Direct-only skills are discoverable without being globally injected into every prompt.

Offline context fixtures under `evals/fixtures/context/built-in-skills/` measure global versus direct-only context impact and direct invocation behavior. They are fixture-only and do not require a live provider.
