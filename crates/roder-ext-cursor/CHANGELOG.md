## 0.1.1 (2026-06-15)

### Fixes

#### Prevent Cursor protobuf decoder panics on malformed payloads

Reject overlong varints and out-of-bounds protobuf fields as decode errors so
unexpected Cursor frames do not stop the agent turn.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
