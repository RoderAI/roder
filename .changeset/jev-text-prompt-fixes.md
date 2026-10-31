---
roder-ext-jev: patch
---

# Relative dates, form-mode rule and text-helper routing

Today's date given to Jev and the text helper now lists the next seven days with their weekdays, and the text helper is told to look a relative day ("next Monday") up there rather than count it, and to write a date as a date, never as a weekday. The next-action rules choose a form's mode (tab, trip or account type) before typing and set requested filters before submitting. Text-helper requests to OpenRouter carry `provider: {sort: "latency", require_parameters: true}` (dropped for the run after a 404 says no endpoint of the model takes every parameter), and a reply cut off at the output cap (`finish_reason: length`) is asked once more with four times the cap. A code fence around or after the text helper's JSON reply is ignored rather than failing the run.
