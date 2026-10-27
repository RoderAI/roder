---
roder-tui: minor
---

# Render local transcript image previews with Kitty and Sixel

Display attached, viewed, and generated local images in clipped transcript layout
rows on supported terminals. Decode and encode images on a bounded worker, limit
encoded input and raster sizes, reflow previews on resize, and retain metadata
fallbacks for unsupported terminals or invalid images. Preview pixels remain
separate from copyable message source. Multiplexers require explicit protocol
opt-in, and Kitty cleanup removes only Roder-owned image IDs.
Worker completions wake an idle UI, and popups suspend and clear image placements
before drawing so pixel payloads cannot overpaint dialogs.
Resolve local viewed-image paths against the authoritative thread workspace,
and do not interpret remote or unknown tool origins as host filesystem paths.
