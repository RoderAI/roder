# Copying original response source

Run `/copy` to open the keyboard copy dialog. It starts on the latest assistant
response. Targets include each whole response and its fenced code blocks.

| Key | Action |
| --- | --- |
| `j`, Down, Tab | Next response or code block |
| `k`, Up, Shift+Tab | Previous response or code block |
| Home, `g` | First target |
| End, `G` | Last target |
| PageUp, PageDown | Scroll the selected source preview |
| Enter, `y` | Copy the selected source |
| Escape, `q`, Ctrl+C | Close the dialog |

Whole-response copying includes the original Markdown. Code-block copying
excludes the opening and closing fences, but preserves the body exactly.
Clipboard payloads preserve tabs, trailing spaces, CRLF/LF/CR line endings,
and missing final newlines. Wrapping, code-preview labels, tool controls, and
image placements do not become clipboard text.

Copy targets are tied to source-message identities and fence positions rather
than screen rows. Resizing the terminal, appending streamed text, or prepending
older history does not move the selection to an unrelated message. The preview
refreshes as source text streams; clipboard input is captured when Enter is
pressed. An unfinished streaming fence extends to the current end of its source.

Copy mode pauses queued prompt dispatch and consumes composer input until it is
closed. Approval and user-input dialogs take precedence. Clipboard success is
shown only after the external clipboard command completes. A failure leaves the
selected source available for retry and shows the error. Closing copy mode does
not cancel an already requested clipboard operation.

The dialog uses the existing `#dialog` surface and semantic border, accent,
text, muted, and error theme roles; it does not introduce unthemeable colors.
