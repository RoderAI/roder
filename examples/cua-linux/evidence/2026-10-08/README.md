# Native Linux desktop evidence

The calculator and original GTK fixtures remain in their `native-*` folders.
The full desktop follow-up uses the same pinned Cua 0.34.0 and built public
Roder runtime, with XFCE/X11, Thunar and LibreOffice Writer.

| Run | Authoritative result |
| --- | --- |
| full-model-a | [Audit passed](full-model-a/audit.json); 64 live model requests, 63 with image blocks; original report passed |
| full-model-b | [Audit passed](full-model-b/audit.json); 58 live model requests, 57 with image blocks; original report rejected reused Writer window identity |
| full-input | [All 16 checks passed](full-input/report.json); two owned recovery desktops deleted and verified TERMINATED |

Both model workflows used only Cua tools to launch apps through the GUI,
navigate folders, select/Shift-drag a note, create and save an ODT report,
close it and reopen it from File Manager. Independent read-only grading
verified the moved file's exact bytes and each ODT's unique run marker and
heading. The profile and package versions are recorded in each report.

The repeat's original report is retained unchanged. LibreOffice closed the
document to Start Center and reopened it in the same native window. The
current audit checks ordered close, File Manager open and fresh Writer
observation; it passes without replaying input. Its reopening used a freshly
grounded Enter after double-clicking did not visibly open the file.
Each model report embeds its full original tool trace; there is no duplicate
trace file in these two evidence folders.

The first workflow's owned desktop was initially retained with a 3-hour TTL
for live viewing. It was subsequently retired when the browser desktop replaced
it; the verified TERMINATED receipt is in browser-model-final/retired-sandboxes.json. The repeat desktop was subsequently
used by the full-input fixture and its TERMINATED receipt is in that report.
Milestone and final images are actual Cua PNGs from these owned desktops.

Exploratory failures are not acceptance passes: the initial 1024×768
mini-model run copied the note and could not finish an oversized Save dialog;
the first unselected Shift-drag on the larger desktop had no observed move.
The qualified workflow uses 1920×1080, selection before Shift-drag, current
window discovery after closed targets, and preserved plain-text refusals.

The [browser follow-up](browser-model-final/README.md) passed through public
Roder and a live model with Chrome, a synthetic signed-in local profile,
semantic browser snapshots, Unicode form input, trusted click and native
address-bar navigation. Its independently graded report and ordered audit are
preserved; that desktop replaces the previous viewer and is explicitly retained
with its own bounded TTL.
