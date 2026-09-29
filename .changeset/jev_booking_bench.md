---
roder-ext-jev: minor
---

# Jev fixes from a live booking benchmark

A new live benchmark (`scripts/jev-booking-bench.sh`, graded by
`scripts/jev_booking_grade.py`) runs a restaurant-booking request through the
installed `roder` binary and grades where it got, what the answer said, that
no commit, sign-in or personal-data step was taken, and that the calls stayed
in one tab. Its runs led to these changes:

- `tab: "new"` right after a call whose page refused access loads the url in
  that tab instead of leaving the refused page open beside a new one; the
  result says why (`tab_note: "navigated"` with a `tab_detail`).
- After `access_denied` the hint says the task is not finished, to go on at
  another site with its url in the same tab, and that switching sites needs
  no confirmation; the tool description says the same.
- The `url` description says the thread's first call needs one, and a first
  call without one says what to send instead of only refusing.
- A heading that titles one card of a list no longer names controls after the
  list (a results map's links were listed as the last card's options).
- In a session tab that already shows a page, a load Chrome abandons
  (`net::ERR_ABORTED`, not a download) is tried once more.
