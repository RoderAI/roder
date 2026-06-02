# Roder Version Control Extensions

Roder exposes version control through `VcsProvider` implementations registered in
the extension registry as `ProvidedService::VersionControlProvider`. The default
distribution installs the bundled git provider with provider id `git`.

Providers should implement the canonical workflow surface in
`roder_api::version_control`:

- detect an active workspace and return a deterministic claim
- report status, active line of work, base information, changed files, and
  capability metadata
- read bounded changed-content pages for provider-relative paths
- expose mutation capabilities for selection, snapshot creation, restore, line
  switching, and sync only where they are safe
- return `VcsError` variants for unavailable providers, unsupported operations,
  invalid paths, dirty workspaces, command failures, and provider-native-only
  behavior

Provider-specific extras are modeled through capability metadata, including
`ProviderNative` states and provider namespaces. There is no separate
`vcs/extras/list` discovery method in the canonical app-server surface.

Provider history snapshots are distinct from Roder thread/session checkpoints.
For git, `vcs/snapshot/create` maps to a commit. Other providers may map it to a
jj change finalization, SVN commit, or a provider-native durable history point.

App-server clients should use `vcs/*` methods. The older `git/*` review methods
are not part of the canonical contract.
