---
roder-ext-jev: patch
---

# Audit pending activations before experimental Jev completion

Add opt-in `JEV_PROMPT_PROFILE=activation`, extending `dom_targets` with a
text-only completion check. Separate typed judgments assess whether the offered
conditional click is required, already performed and ready. Conservative support
can recover a pending click while retaining its original target probabilities
and irreversible-action gate. Corrected operation confidence records derived
support, and usage includes the additional request. Default behavior is unchanged.
