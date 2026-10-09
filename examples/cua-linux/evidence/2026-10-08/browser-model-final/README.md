# Public Roder Cua browser qualification

The preserved original report passed. A live `gpt-5.4` model used only Cua tools
through the built public Roder app-server on the full XFCE/X11 Blaxel desktop.
There were 36 completed/failed public tool executions and 33 model requests;
32 requests included actual image blocks. The current read-only audit also
passes, requiring the ordered native Ctrl+L, URL typing, Enter and later receipt
observation rather than accepting an arbitrary keypress as browser-chrome proof.

The model signed into the local test website with native Cua input, attached
that running Chrome profile, bound the exact native window, navigated using
browser tools, typed Unicode and a unique marker into the form, submitted with
a trusted foreground browser click, revisited the receipt through the native
address bar, captured the desktop and ended the Cua browser session. The
independent read-only website oracle confirms the authenticated submission,
exact title/notes, later authenticated receipt reads and retained profile.

Chrome is 155.0.8059.39. Its official Debian package SHA256 is recorded in the
report; Cua Driver is checksum-pinned 0.34.0. The browser has renderer
accessibility plus ACCESSIBILITY_ENABLED=1, and runs under the actual desktop
user/display/D-Bus prefix. Chrome's process sandbox is unavailable in this test
container, so only this disposable fixture uses --no-sandbox. The production
extension does not add this flag.

Automatic driver setup initially returned browser_wrong_target_refused with
side effects because the exact setup page was not ready. The model inspected
the current native Chrome tabs, selected the setup page and its exact
accessibility checkbox, then prepared/bound again. No input was automatically
replayed or silently changed to another route. Browser attachment reports the
same native PID, no restart, no new profile and no copied profile data. Two
schema-invalid native clicks were rejected before input and corrected by the
model. All refusals remain in the original trace.

An earlier exploratory run without the forced native accessibility bridge
signed in natively but could not attach and is not counted as a pass. That owned
sandbox and the previous viewer desktop are verified TERMINATED in
retired-sandboxes.json. The qualified browser desktop is explicitly retained
for the user's live viewer with a 3-hour lifetime from creation, as recorded in
retention.json. Its Cua browser session ended; Chrome's debug setting remains
inside the retained disposable sandbox. Live capture stops after one hour.

The report embeds the original full tool trace; PNGs are actual Cua captures.
No browser cookies, real credentials or copied personal profile are included.
