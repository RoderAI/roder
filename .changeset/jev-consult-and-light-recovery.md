---
roder-ext-jev: minor
---

# Ask the text model before an unsure repeat; opt-in consult and light recovery

When Jev is unsure and about to repeat the click it just made, which the repeat guard would stop, the text model is now asked first (`JevConsultant`, `JevEngineConfig::with_consultant`, with Roder's text helper as the consultant). If it names the same control, the click goes ahead. If it names another control, that control is acted on instead, still gated. If it names nothing, the guard stops as before. This lets a feed's second "Load more" through: `mock_infinite_scroll` used to end `blocked` with 40 of 60 items. A repeated commitment, a repeat that adds something (such as "Add to cart") or posts a form, and a twin still stop.

Two fastbrowse tiers ship off by default, because they did not measurably help on the live corpus or MiniWoB++:

- `JEV_CONSULT_BELOW=0.55` (`with_consult_below`) consults before any step that acts under that call confidence. It may act on another observed control, which is still gated, with no answers and no confidence.
- `JEV_LIGHT_RECOVERIES=2` (`with_light_recoveries`) gives a stalled, covered or looping run up to that many one-step recoveries before it ends `blocked`. A recovery spends the stall's evidence.

`JEV_CONSULT=0` turns the consultant off entirely. The consultant is never offered a cookie-banner choice. Its calls are counted as text calls with their usage, and the run's `recoveries` record what each one did.
