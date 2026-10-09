---
roder-core: minor
---

# Core stops advertising screenshots to engines that never receive them and routes browser tools

- Engines that cannot receive tool-result images get a request copy without the image plus one notice line. Screenshot-only tools are hidden from them, and prompt accounting charges them no image tokens. The stored transcript is unchanged.
- When two or more browser tool families (`jev_browse`, `chrome_*`, `browser_use_*`, `webwright.*`) are advertised, the developer instructions gain a Browser Tool Routing block that says which family to use for which job.
