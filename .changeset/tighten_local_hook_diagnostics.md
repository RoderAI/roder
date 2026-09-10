---
roder-core: patch
---

# Tighten local hook output, matchers, and inert handlers

- `additionalContextLimit` is honoured instead of parsed and dropped, and output
  is trimmed to a byte budget on a char boundary. The cap counted chars, so
  multi-byte output reached four times its intended size.
- An unparseable `matcher` reports itself as a failed hook run rather than
  silently disabling its hook.
- `prompt` and `agent` handlers record as `skipped`, not `success`, and can no
  longer be read as a permission decision.
- `hooks.json` is parsed once per file version rather than on every dispatch,
  keyed on modified time and length.
