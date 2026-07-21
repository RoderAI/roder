## 0.1.2 (2026-07-21)

### Fixes

#### Read lifecycle state without loading full threads

Thread stores can now load persisted extension state directly. Lifecycle-only
reads use that seam, so metadata-only thread reads do not need to project a
full event, turn, and item snapshot.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
