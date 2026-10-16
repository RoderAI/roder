---
roder-ext-chrome: patch
---

# Bring a headless tab to the front before its screenshot

Headless Chrome can leave `Page.captureScreenshot` waiting for a frame that an idle page never draws, so the screenshot timed out. A headless session now calls `Page.bringToFront` before each capture.
