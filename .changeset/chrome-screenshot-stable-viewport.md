---
roder-ext-chrome: patch
---

# Keep the browser viewport stable during screenshots

Direct CDP screenshots no longer temporarily shrink and restore the page on
Retina displays. Capture the existing viewport and resize the returned image
locally, preserving one image pixel per input coordinate and secret-field masks.
