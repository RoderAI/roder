---
roder: patch
roder-core: patch
roder-tools: patch
---

# Initialize discovery before agents use it

Build the tool and skill discovery catalog during CLI startup instead of waiting
for a discovery API request. Missing catalogs now return an actionable fallback,
and agents skip redundant discovery searches or retries when discovery is
unavailable. Catalog tools also honor the same config-directory precedence as
the catalog builder.
