---
roder-ext-jev: minor
---

# Jev stops before clicking the wrong twin row

An unsure delete (or other commit-named click) on a twin control right after a click on the same label that changed the page now ends the run done without clicking, instead of clicking the wrong twin. The run result gains `suppressed_click` (`JevSuppressedClick`, `JevSuppressedKind`) and the result text a "Not clicked:" line. The old same-control rule also reports what it held back.
