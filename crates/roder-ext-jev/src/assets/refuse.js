// Refuses a cookie banner before Jev reads a document (Jev's own code).
// Called with consent.js. DuckDuckGo's autoconsent (MPL-2.0, vendored
// unmodified under autoconsent/), when Jev injected it, goes first on the
// consent platforms it knows; consent.js is the fallback for banners it does
// not recognise. The two never both act on one document, in this order:
// 1. While autoconsent is opting out (it found an open popup and runs its
//    steps), wait for it, up to 3 s; consent.js does not run meanwhile.
// 2. An opt-out autoconsent reports is returned, once per document, and
//    consent.js is then never run in that document.
// 3. While autoconsent's prehide style hides an element (a banner of a
//    platform it knows, kept hidden until it acts, for 2 s at most),
//    consent.js waits for a later read.
// 4. Otherwise consent.js runs, once per document. If it clicks, autoconsent
//    is told to take no action in this document (its autoAction is cleared,
//    so a popup it finds later only waits for a signal that never comes).
// Every check runs in one task after the last await, so autoconsent cannot
// start between the check and the click. Only the top document is read:
// autoconsent working inside a frame is not reported.
// Returns null when there is nothing new, {autoconsent: platform} for
// autoconsent's opt-out, else consent.js's {clicked: label} or {skipped: why}.
(async (consent) => {
  const BUSY = ['openPopupDetected', 'runningOptOut'];
  const standalone = window.autoconsentStandalone;
  const state = () => standalone?.instance?.state;
  const busy = () => BUSY.includes(state()?.lifecycle);
  const deadline = performance.now() + 3000;
  while (busy() && performance.now() < deadline) await new Promise(r => setTimeout(r, 50));
  if (Array.isArray(standalone?.messages) && !window.__jevAutoconsentReported) {
    const done = standalone.messages.find(m => m?.type === 'optOutResult' && m.result === true);
    if (done) {
      window.__jevAutoconsentReported = true;
      window.__jevConsentChecked = true;
      return {autoconsent: String(done.cmp || 'a consent platform').slice(0, 80)};
    }
  }
  if (window.__jevConsentChecked || busy()) return null;
  // An element the prehide style hides now: a banner autoconsent may refuse.
  const prehidden = () => {
    const sheet = state()?.prehideOn && document.getElementById('autoconsent-prehide')?.sheet;
    if (!sheet) return false;
    for (const rule of sheet.cssRules) {
      try { if (rule.selectorText && document.querySelector(rule.selectorText)) return true; } catch {}
    }
    return false;
  };
  if (prehidden()) return null;
  window.__jevConsentChecked = true;
  const outcome = consent();
  if (outcome?.clicked !== undefined && standalone) {
    try { standalone.instance.config.autoAction = null; } catch {}
  }
  return outcome;
})
