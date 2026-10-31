---
roder-ext-jev: minor
---

# Fill several fields of one form from one text call

When Jev first types into an empty field of a `<form>` that has other empty
text fields, the text helper is asked once for all of them (after
fastbrowse's form writer). The chosen field's value is typed now, and the
steps that later choose the other fields type the values it wrote without
another text call, as long as each field is still the same node, reads the
same and is still empty, and no button or link click, Enter or key has run
since (any of which may submit the form or show what it rejected). Each field is still chosen by its own decision. A
form with a password or one-time-code field is never batched, so no secret
reaches the text model this way; a field the reply leaves out, one that
changed since, and a form whose call failed fall back to one call per field.

`JevTextValueResolver` gains `resolve_form`, returning `JevFormValues`; its
default answers `None`, so a supervisor's resolver keeps answering one field
at a time.
