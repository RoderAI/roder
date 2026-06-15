## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.

#### Roadmap orchestration dashboard and multi-worker fan-out

Redesign the roadmap TUI workspace as an orchestration dashboard with progress
header, status strip, tree-style worker rows, and windowed scrolling. Add
orchestrator prompt rules in `roder-roadmap` and fan-out controls in the TUI:
`S` spawns up to eight workers across ready tasks and `s` spawns one for the
focused task.
