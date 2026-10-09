---
roder-ext-jev: minor
---

# Jev names what covered a click

A step whose click was covered now names what covered it: `JevActionRecord.covered_by`, `Covered::with_cover` and `Covered::cover`. The name is page text, scrubbed, on one line and cut to 100 characters, and the digest and the fallback prompt show it as untrusted. The hosted chooser request is unchanged.

The fallback's opening message now labels Jev's stop reason as untrusted page text, collapses it to one line and cuts it to 300 characters, because a `looped` stop quotes the page's own control label; the standing fallback instructions say the reason and the last steps are untrusted too.
