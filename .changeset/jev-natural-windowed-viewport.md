---
roder-ext-jev: patch
roder-ext-chrome: patch
---

# Keep Jev screenshots from resizing visible Chrome windows

Jev now uses a windowed browser's natural viewport and display scale instead of
forcing a fixed non-Retina viewport. Its screenshots share the direct CDP image
normalization, keeping CSS coordinates accurate without resizing Chrome during
capture. Headless Jev sessions retain their fixed viewport.
