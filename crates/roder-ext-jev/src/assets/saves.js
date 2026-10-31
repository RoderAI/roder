// Counts the page's own attempts to save a file (Jev's own): a click on a
// link that names a download Chrome will save, whether
// a person's click reaching the document or a script's click on a link it
// never attached (FileSaver's `saveAs` dispatches one). Chrome announces
// such a download a moment later, sometimes after the page has settled, so
// a count above zero tells Jev to wait for the announcement. Installed on
// every document; idempotent. `window.__jevSaves` is read and reset after
// each action.
(() => {
  if (window.__jevSaving) return;
  window.__jevSaving = true;
  window.__jevSaves = 0;
  // Chrome honours `download` only on a link of the page's own origin, or
  // one to a blob: or data: address; elsewhere the link navigates.
  const saves = node => node instanceof HTMLAnchorElement && node.hasAttribute('download')
    && (/^(blob|data):/i.test(node.href || '') || node.origin === location.origin);
  const count = () => { window.__jevSaves = (window.__jevSaves | 0) + 1; };
  addEventListener('click', event => {
    if (event.composedPath().some(saves)) count();
  }, true);
  const anchor = HTMLAnchorElement.prototype;
  const click = anchor.click;
  anchor.click = function () {
    if (!this.isConnected && saves(this)) count();
    return click.apply(this, arguments);
  };
  const dispatch = EventTarget.prototype.dispatchEvent;
  anchor.dispatchEvent = function (event) {
    if (event?.type === 'click' && !this.isConnected && saves(this)) count();
    return dispatch.call(this, event);
  };
})();
