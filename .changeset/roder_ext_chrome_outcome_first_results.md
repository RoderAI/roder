---
roder-ext-chrome: minor
---

# chrome tools return outcome-first results, a working chrome_select, and a safer native computer

- `chrome_select` now works on the Roder Desktop browser: `{tabId?, ref?, selector?, value}` with `value` required and exactly one of `ref` or `selector`. The option matches by value, then by visible text, and a miss lists the options. A disabled, ambiguous or non-select target is refused, and a page that restores the old value is reported as a failure. The shared direct select (also used by `jev_tab_select`) matches value before text, refuses ambiguous and disabled options, and sets `is_error` when the page puts the old value back.
- `chrome_page_text` and `chrome_highlight` answer "not supported on the Roder Desktop browser" instead of "not connected".
- `chrome_*` snapshot, navigation and page-action results on the paired extension render as an outcome line, then controls (marked new or changed), then text, within the 24,000-character cap. Text is cut first and controls last, with counts. A snapshot of only some sections says which it did not read and does not replace the fuller page used for the next outcome. When an extension build returns an action reply without an observation, one `page/snapshot` is taken after 150 ms. `chrome_page_snapshot` no longer lists the `boxes` include.
- Desktop `chrome_click`, `chrome_type`, `chrome_keypress`, `chrome_scroll` and `chrome_select` lead with the untrusted-content label and the same outcome sentence, and the Desktop look marks text it cut.
- Native computer batches return `computer_notes` (page, HTTP status, new tab, dialogs, remapped chords, actions that did not run). Every batch carries a screenshot, with a placeholder when none can be taken. Ctrl editing chords are sent as Cmd on Mac pages, a batch stops after a navigation or new tab, and dialog text is scrubbed of typed secrets.
- Covered-target errors from the `chrome_*` tools on the Desktop browser, and from direct type, no longer suggest pressing at x/y coordinates, which those tools cannot do. They point to closing the cover, scrolling, or choosing another target. The cover's name is scrubbed, put on one line, stripped of its own quote marks and capped at 60 characters.
