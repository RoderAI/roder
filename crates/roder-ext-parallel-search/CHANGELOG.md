## 0.1.2 (2026-07-23)

### Features

#### Parallel search + extract web tools

Fix Parallel.ai Search against the current V1 API (`advanced_settings` for
max_results/domain filters), add `parallel_extract` for URL markdown extraction,
auto-install Parallel tools when it is the selected web_search provider, and
inject short Parallel web-access instructions into the developer prompt when
those tools are available.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
