---
roder-ext-jev: minor
---

# Jev's settle waits for the page's own requests and ignores loader class names

After an input, a page that is quiet and unchanged while a fetch or XHR request that input started is still open is no longer read before the response lands: the settle waits for the response and then for quiet again, within the 2.5 s cap, and reports `reason: "response"`. Each tab now enables the Network domain for this. Class and id names such as `spinner`, `loader` or `skeleton` no longer count as loading indicators; only `aria-busy`, `role="progressbar"` and an unvalued `progress` hold the settle, so pages that merely name their components after loaders no longer pay a 1.5 s wait on every step.
