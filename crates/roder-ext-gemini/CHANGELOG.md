## 0.1.2 (2026-08-20)

### Fixes

#### Accept more tool schemas in Gemini

Remove unsupported `uniqueItems` fields and avoid converting non-string constants into invalid Gemini enum values.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
