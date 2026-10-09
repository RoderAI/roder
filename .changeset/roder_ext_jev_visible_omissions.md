---
roder-ext-jev: minor
---

# jev_browse says what it did not offer Jev, and reports checkbox state correctly

`jev_browse` results now say what the page held that Jev was not offered: a counts-only "Not offered to Jev" header line and an `omitted` field (`controls` the snapshot left out, counted per control instead of per action, and `options` past the 255 a choice takes, such as a 300-option select). Checkboxes, radios and switches now report `checked` true/false in the result's controls and read [checked]/[unchecked] in the options list, instead of the HTML value "on". `JevControl` gains `checked`, `JevRunResult` gains `omitted`, and `JevOmitted` is new public API.
