---
roder-ext-jev: patch
---

# Read frames of another process on kept sessions, within a 5 s deadline each

Each tab's setup turns on `Target.setAutoAttach` for iframes, and the connection keeps the sessions Chrome attaches to frames of another process (`cdp/frames.rs`), so reading such a frame's text is one evaluate instead of a target listing, an attach and a detach (about 90 ms down to 4 to 8 ms per read on the fixture site; pages without such frames drop `Target.getTargets` from every observation). Each frame's read now has its own 5 s deadline instead of the connection's 30 s, and a frame of another process that missed it gets 0.5 s on later reads for 30 s, so an advertisement whose renderer stopped answering costs one observation 5 s, not every observation 30 s. Frames stay read-only.
