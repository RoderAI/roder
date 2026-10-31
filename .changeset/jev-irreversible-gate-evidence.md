---
roder-ext-jev: major
---

# Jev's irreversible-action gate reads the page's evidence, stops password changes and forbidden downloads

Breaking: `JevDecision` and `JevDecisionRecord` gain `downloads_forbidden: Option<f64>`, so a struct literal of either must set it (`None` when the gate did not ask).

With the opt-in gate on (`JEV_CONFIRM_IRREVERSIBLE=1`), the shortlist of actions it asks about now reads more than the label: the last segment of a link's address (a one-click `/unsubscribe` link labelled "Stop these emails"), the heading or row a control sits under (a dialog's "Yes, I'm sure" under "Delete the Atlas project?"), agreement labels, and buttons that post their form, with the label-and-link evidence asked about first. The commitment words gain `deactivate`, `withdraw`, `terminate`, `renew`, `upgrade`, `downgrade`, `sell`, `bid`, `approve` and the same words in German, French, Spanish, Portuguese, Italian and Dutch (`bestellen`, `commander`, `comprar`...). Each question now quotes the element's label, where it sits, where a link goes and the form a submit sends, and observed actions carry `href`, `download` and `submits` for it. Play, pause and mute controls are never asked about unless they name a committed noun.

Two new stops need no answer about the action: a submit or Enter in a form that sets a new password ends the run `needs_confirmation` unless the call is authorized and confident, and a click the page says saves a file (a `download` attribute, or a link to a `.zip`, `.exe`, `.csv`...) ends it when one more question, asked of the goal alone, says the task rules out downloading, whatever the authorization.

Past the 8-question cap, only an Enter or a click whose label names a commitment fails closed; a click shortlisted only by its address, heading, row or posting form that the cap left unasked is dispatched, so a page of many "Add to cart" forms cannot stop a run. A sign-in wall now compares form ids as the snapshot gives them (numbers), so a missing value in a sign-up form is `needs_input` rather than `needs_login`.
