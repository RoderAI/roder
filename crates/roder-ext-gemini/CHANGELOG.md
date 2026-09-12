## 0.1.3 (2026-09-12)

### Fixes

#### Add Gemini 3.7 Flash and Grok 4.6 to provider model catalogs

Expose Gemini 3.7 Flash and Grok 4.6 through native, Cursor, xAI, SuperGrok,
and OpenRouter integrations with provider-specific context windows and
reasoning controls. Retire Grok 4.5 and Grok Build from active catalogs.

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
