---
roder-ext-jev: major
---

# Jev saves downloads, waits out pages that clear themselves, and reads the HTTP status from the browser

Breaking: `JevRunResult` gains `downloads` and `JevActionRecord` gains `downloads` (both `Vec<JevDownload>`: the file's `name`, `url`, `path` where the browser saved it, `size_bytes`, and `incomplete` / `repeated` flags). A link to a file or a button that saves one used to read as a click that changed nothing, so the model clicked the export again until the stall rule stopped the run. Jev's browser now saves the downloads of its tabs' browser context under `<temp>/jev-downloads` (each finished file in `kept/<guid>/<name>`), waits after an action for the files it started, and the next observation reports each one, with the start of a readable file's text, before the page text. A new file counts as the step's change; the same file saved again (same name and content) does not, and the stall rule still ends a run that keeps saving it. The tool's text lists the files with where they were saved.

An opening page that may clear itself (a challenge such as "Just a moment...", or a 503, with nothing to act on) is now watched for up to 12 s, read again each time it changes, before it is classified, so a bot check that moves on or a busy site that retries no longer ends the call `access_denied` or costs decisions on an empty page. A plain 403 "Access Denied" or a 429 is not waited on. A 503 with a challenge phrase and nothing to act on is now an access block. A start page that still shows nothing after the empty-page reads, served without an error status, is loaded once more. `JevBrowser` gains two default methods for this, `reload` and `wait_for_change`.

The page's HTTP status (`JevPageFacts.http_status`) is now the one the browser's Network events gave the committed document, so a file the page linked to, or a page script that shadows `performance`, no longer changes it.
