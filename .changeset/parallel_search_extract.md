---
roder-ext-parallel-search: minor
roder-extension-host: patch
roder-core: minor
roder: minor
---

# Parallel search + extract web tools

Fix Parallel.ai Search against the current V1 API (`advanced_settings` for
max_results/domain filters), add `parallel_extract` for URL markdown extraction,
auto-install Parallel tools when it is the selected web_search provider, and
inject short Parallel web-access instructions into the developer prompt when
those tools are available.
