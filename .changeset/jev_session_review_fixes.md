---
roder-ext-jev: patch
---

# Jev session and result fixes from review

- A continued call no longer adopts, or closes, tabs the user opened from
  Jev's tab between calls: every page open when the call re-attaches is
  recorded as seen, and only tabs this call's own inputs open are followed.
- A session leaves the registry (close, idle sweep, eviction) only under its
  own lock and marked closed; a call that was waiting for it moves to the
  thread's new session instead of opening tabs nothing would ever close.
- A `new` tab asked for right after `access_denied` loads in the refused tab
  only while that tab is still open; otherwise the call opens the new tab it
  asked for rather than loading over the tab before it.
- With `JEV_ALLOWED_ORIGINS` set, a call whose tabs are gone does not reopen
  a last page outside those origins; it ends `blocked` without loading it.
- The address a continued call finds its tab at is scrubbed of typed secrets
  before it is compared and reported, so a password in a GET form's address
  never reaches `session.moved_to` or the result text.
- Frame text, frame origins and every control's `section` are scrubbed of
  typed secrets like the rest of the page.
- The step fingerprint counts frames of another origin by origin, not text,
  so a rotating ad frame cannot defeat the stall rule.
- A `/sorry/` address counts as a bot challenge only with a refusal status,
  a refusal phrase or nothing to act on.
- The result text defuses any run of three or more dashes, ASCII or
  look-alike, so page text cannot draw a marker line; and when a flow runs
  past midnight it says the user's "tonight" is the date the session began
  on (`session.began_on`).
- The tool's goal example and wording no longer carry the booking
  benchmark's task, and the advice to move to another site after
  `access_denied` without asking applies only when the user named no
  particular site.
- The booking benchmark grader passes only runs that reached the reservation
  panel, and reads its date, party size and time off the panel.
