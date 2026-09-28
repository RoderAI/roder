// Jev's one setting for DuckDuckGo's autoconsent (Jev's own code; the
// vendored bundle in autoconsent/ is unmodified). It runs in every document
// right after the bundle, which has just initialised itself with
// heuristicMode "tier2": beyond the rules for known consent platforms, its
// heuristics then also press the "OK" of a banner that only acknowledges
// and the single "Accept" of one that offers nothing else. Jev refuses and
// never accepts, so the heuristics are held to "reject": a popup they find
// is acted on only through a refusal button. Detection starts later (on
// DOMContentLoaded, when the page is idle), so this lands first.
(() => {
  try {
    const config = window.autoconsentStandalone?.instance?.config;
    if (config) config.heuristicMode = 'reject';
  } catch {}
})();
