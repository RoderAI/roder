// Roder's direct tools, inside the page: installed once per document as
// window.__roderDirect, and called through it. Everything here reads the
// page or resolves a point; input itself is sent as real DevTools events.
// Refs (e1, e2, ...) name elements a look listed; an element keeps its ref
// for as long as it is in the document, across looks, so a ref read before
// an action still names the same element after it. A secret field (a
// password, a one-time code, or a field masked as one) is never read: only
// whether it holds anything.
(() => {
  if (window.__roderDirect?.v === 1) return;
  const ROLES = ['button','link','checkbox','radio','switch','tab','menuitem','menuitemradio',
    'menuitemcheckbox','option','combobox','textbox','searchbox','slider','spinbutton','gridcell',
    'treeitem'];
  const INTERACTIVE = 'a[href],button,input,textarea,select,summary,[contenteditable=""],' +
    '[contenteditable="true"],[onclick],[draggable="true"],[tabindex]:not([tabindex="-1"])' +
    ROLES.map(r => `,[role="${r}"]`).join('');
  // Whose inside is part of their own click.
  const WHOLE = 'a[href],button,summary,label,[role="button"],[role="link"],[role="option"],' +
    '[role="menuitem"],[role="tab"],[role="checkbox"],[role="radio"]';
  const FIELD = 'input,textarea,select,[contenteditable=""],[contenteditable="true"]';
  const GRAPHIC = 'canvas,svg,video,iframe';
  const S = { v: 1, nodes: new Map(), ids: new WeakMap(), next: 1, secrets: new WeakSet(),
    masks: [] };
  const refOf = el => {
    let ref = S.ids.get(el);
    if (!ref) { ref = 'e' + S.next++; S.ids.set(el, ref); }
    S.nodes.set(ref, el);
    return ref;
  };
  const cut = (s, n) => {
    s = String(s ?? '').replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n - 1) + '…' : s;
  };
  const style = el => getComputedStyle(el);
  const box = el => {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    const s = style(el);
    if (s.visibility === 'hidden' || s.display === 'none') return null;
    if (Number(s.opacity) === 0 && el.tagName !== 'SELECT') return null;
    return r;
  };
  const secret = el => {
    if (S.secrets.has(el)) return true;
    if (el?.tagName !== 'INPUT') return false;
    if (el.type === 'password') return true;
    if (/(password|one-time-code)$/i.test(el.getAttribute('autocomplete') || '')) return true;
    const masked = style(el).webkitTextSecurity;
    return !!masked && masked !== 'none';
  };
  const text = el => el.innerText ?? el.textContent ?? '';
  const label = el => {
    const aria = el.getAttribute('aria-label');
    if (aria?.trim()) return aria;
    const by = el.getAttribute('aria-labelledby');
    if (by) {
      const t = by.split(/\s+/).map(id => text(document.getElementById(id) || {})).join(' ');
      if (t.trim()) return t;
    }
    if (el.labels?.length) return [...el.labels].map(text).join(' ');
    const typed = el.tagName === 'INPUT' && ['submit', 'button', 'reset'].includes(el.type);
    return el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('alt') ||
      (typed ? el.value : '') || (el.matches(FIELD) ? '' : text(el)) ||
      el.querySelector('img[alt]')?.alt || el.querySelector('svg title')?.textContent || '';
  };
  const pointer = el => {
    if (style(el).cursor !== 'pointer') return false;
    const parent = el.parentElement;
    return !parent || style(parent).cursor !== 'pointer';
  };
  const role = el => el.getAttribute('role') || null;
  const kind = el => el.matches(GRAPHIC) ? 'graphic' : el.matches(FIELD) ? 'field' : 'control';
  const describe = (el, r, ref) => {
    const item = { ref, tag: el.tagName.toLowerCase(), kind: kind(el), label: cut(label(el), 80) };
    if (role(el)) item.role = role(el);
    if (el.type && el.tagName === 'INPUT') item.type = el.type;
    if (secret(el)) { item.secret = true; item.filled = !!el.value; }
    else if (el.tagName === 'SELECT') {
      item.value = cut(el.selectedOptions?.[0]?.text, 60);
      item.options = [...el.options].slice(0, 12).map(o => cut(o.text, 40));
    } else if (el.matches('input,textarea') && !['checkbox', 'radio'].includes(el.type)) {
      if (el.value) item.value = cut(el.value, 60);
    } else if (el.isContentEditable && text(el).trim()) item.value = cut(text(el), 60);
    if (el.checked !== undefined && ['checkbox', 'radio'].includes(el.type)) item.checked = el.checked;
    const aria = el.getAttribute('aria-checked') || el.getAttribute('aria-selected');
    if (aria) item.checked = aria === 'true';
    if (el.getAttribute('aria-expanded')) item.expanded = el.getAttribute('aria-expanded') === 'true';
    if (el.disabled || el.getAttribute('aria-disabled') === 'true') item.disabled = true;
    if (el.getAttribute('draggable') === 'true') item.draggable = true;
    item.x = Math.round(r.left); item.y = Math.round(r.top);
    item.w = Math.round(r.width); item.h = Math.round(r.height);
    if (r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) item.offscreen = true;
    return item;
  };
  const status = () => {
    try { return performance.getEntriesByType('navigation')[0]?.responseStatus || null; }
    catch { return null; }
  };
  const focused = () => {
    let el = document.activeElement;
    while (el?.shadowRoot?.activeElement) el = el.shadowRoot.activeElement;
    return el && el !== document.body && el !== document.documentElement ? el : null;
  };
  // The control a point or an element belongs to, and what a gate needs to
  // know about it.
  const control = el => {
    for (let x = el; x && x !== document.body; x = x.parentElement) {
      if (x.matches(INTERACTIVE) || pointer(x)) return x;
    }
    return null;
  };
  // A region laid over the page that talks about cookies or consent.
  const CONSENT = /cookie|consent|gdpr/i;
  const inConsent = el => {
    for (let x = el, i = 0; x && x !== document.body && i < 12; x = x.parentElement, i++) {
      const s = style(x), named = (x.id || '') + ' ' + (typeof x.className === 'string' ? x.className : '');
      const region = x.tagName === 'DIALOG' || ['fixed', 'sticky'].includes(s.position) ||
        ['dialog', 'alertdialog', 'region', 'banner'].includes(role(x)) || /cookie|consent/i.test(named);
      if (region && CONSENT.test((x.innerText || '').slice(0, 2000))) return true;
    }
    return false;
  };
  // A frame the page cannot see into (another site's): what a press there
  // hits is unknown, so it is reported as a press into that frame.
  const frameProbe = el => {
    let origin = '';
    try { origin = new URL(el.src, location.href).origin; } catch {}
    return { tag: el.tagName.toLowerCase(), role: null, control: true, frame: true,
      label: cut(el.getAttribute('title') || el.getAttribute('aria-label') || origin, 80),
      href: null, secret: false, field: false, submit: false, form_labels: [],
      secret_form: false, consent: false };
  };
  const inner = el => {
    try { return el.contentDocument || null; } catch { return null; }
  };
  const probeOf = el => {
    if (!el) return { tag: null, control: false };
    if (el.tagName === 'IFRAME' || el.tagName === 'FRAME') return frameProbe(el);
    const c = control(el);
    const at = c || el;
    const form = at.form || at.closest('form');
    const submits = form ? [...form.querySelectorAll(
      'button:not([type=button]):not([type=reset]),input[type=submit],input[type=image]')]
      .map(b => cut(label(b), 60)).filter(Boolean).slice(0, 6) : [];
    const secretForm = !!form && [...form.querySelectorAll('input')].some(i => secret(i) && i.value);
    return {
      tag: at.tagName.toLowerCase(), role: role(at), control: !!c, label: cut(label(at), 80),
      href: at.closest('a[href]')?.href || null, secret: secret(at),
      field: at.matches(FIELD), submit: !!form && (at.type === 'submit' || (at.tagName === 'BUTTON' && !at.getAttribute('type'))),
      form_labels: submits, secret_form: secretForm, consent: inConsent(at),
    };
  };
  S.look = opts => {
    const max = opts?.max || 120;
    for (const [ref, el] of S.nodes) if (!el.isConnected) S.nodes.delete(ref);
    const seen = new Set();
    const found = [];
    const add = el => {
      if (seen.has(el)) return;
      seen.add(el);
      const r = box(el);
      if (!r) return;
      if (el.matches(GRAPHIC) && (r.width < 24 || r.height < 24 || el.closest(WHOLE))) return;
      // The inside of a button or link is the button's click, unless it is
      // a field of its own.
      const whole = el.parentElement?.closest(WHOLE);
      if (whole && seen.has(whole) && !el.matches(FIELD)) return;
      found.push([el, r]);
    };
    for (const el of document.querySelectorAll(INTERACTIVE + ',' + GRAPHIC)) add(el);
    const all = document.body ? document.body.getElementsByTagName('*') : [];
    for (let i = 0; i < all.length && i < 20000; i++) {
      const el = all[i];
      if (!seen.has(el) && !el.closest(WHOLE + ',' + FIELD) && pointer(el)) add(el);
    }
    const onscreen = ([, r]) => r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth;
    found.sort((a, b) => (onscreen(b) - onscreen(a)) || (a[1].top - b[1].top) || (a[1].left - b[1].left));
    const items = found.slice(0, max).map(([el, r]) => describe(el, r, refOf(el)));
    const f = focused();
    return {
      url: location.href, title: document.title, http_status: status(),
      viewport: { w: innerWidth, h: innerHeight, scroll_y: Math.round(scrollY),
        page_h: Math.round(document.documentElement.scrollHeight) },
      focused: f ? cut(label(f) || f.tagName.toLowerCase(), 60) : null,
      elements: items, omitted: Math.max(0, found.length - max),
      text: (document.body?.innerText || '').replace(/[ \t]+/g, ' ').replace(/\n\s*\n+/g, '\n')
        .trim().slice(0, opts?.text || 3000),
    };
  };
  // Where a ref is on screen now, scrolled into view when it is not; at a
  // fraction of its box (the centre by default). `covered` when another
  // element would take a press there.
  S.point = (ref, fx, fy) => {
    const el = S.nodes.get(ref);
    if (!el?.isConnected) return { gone: true };
    let r = box(el);
    if (!r) return { gone: true };
    if (r.top < 0 || r.left < 0 || r.bottom > innerHeight || r.right > innerWidth) {
      el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
      r = el.getBoundingClientRect();
    }
    const x = r.left + r.width * (fx ?? 0.5), y = r.top + r.height * (fy ?? 0.5);
    const hit = document.elementFromPoint(x, y);
    const mine = !!hit && (hit === el || el.contains(hit) || (hit.shadowRoot && hit.contains(el)) ||
      (el.tagName === 'LABEL' && el.control === hit) || hit.closest?.('label')?.control === el);
    const out = { x, y };
    if (!mine) { out.covered = true; out.by = hit ? cut(label(hit) || hit.tagName.toLowerCase(), 60) : null; }
    return out;
  };
  // Into same-origin frames, to the element the point hits there.
  const probeAt = (doc, x, y) => {
    const el = doc.elementFromPoint(x, y);
    if (el && (el.tagName === 'IFRAME' || el.tagName === 'FRAME')) {
      const sub = inner(el);
      if (sub) {
        const r = el.getBoundingClientRect();
        return probeAt(sub, x - r.left - el.clientLeft, y - r.top - el.clientTop);
      }
    }
    return probeOf(el);
  };
  S.probe = (x, y) => probeAt(document, x, y);
  S.probeRef = ref => probeOf(S.nodes.get(ref));
  S.probeFocused = () => {
    let el = focused();
    while (el && (el.tagName === 'IFRAME' || el.tagName === 'FRAME') && inner(el)) {
      const sub = inner(el).activeElement;
      if (!sub || sub === inner(el).body) break;
      el = sub;
    }
    return probeOf(el);
  };
  // Focus a field by ref and, when asked, select what it holds, so typing
  // replaces it. Whether it is a secret field.
  S.focus = (ref, clear) => {
    const el = S.nodes.get(ref);
    if (!el?.isConnected) return { gone: true };
    el.focus({ preventScroll: false });
    if (clear) {
      if (el.select) el.select();
      else if (el.isContentEditable) {
        const range = document.createRange(); range.selectNodeContents(el);
        const sel = getSelection(); sel.removeAllRanges(); sel.addRange(range);
      }
    }
    return { focused: focused() === el || el.contains(focused()), secret: secret(el) };
  };
  // What covers the focused control at its centre, when something does: a
  // key that presses it would get around that.
  S.focusCovered = () => {
    const f = focused();
    if (!f) return null;
    const r = f.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    const x = r.left + r.width / 2, y = r.top + r.height / 2;
    if (x < 0 || y < 0 || x >= innerWidth || y >= innerHeight) return null;
    const hit = document.elementFromPoint(x, y);
    if (!hit || hit === f || f.contains(hit) || hit.contains(f)) return null;
    if (hit.tagName === 'LABEL' && hit.control === f) return null;
    return cut(label(hit) || hit.tagName.toLowerCase(), 60);
  };
  S.focusedSecret = () => { const f = focused(); return !!f && secret(f); };
  S.markSecret = () => { const f = focused(); if (f) S.secrets.add(f); };
  S.select = (ref, option) => {
    const el = S.nodes.get(ref);
    if (!el?.isConnected) return { gone: true };
    if (el.tagName !== 'SELECT') return { not_select: true };
    const want = String(option).trim().toLowerCase();
    const match = [...el.options].find(o => o.text.trim().toLowerCase() === want) ||
      [...el.options].find(o => o.value.toLowerCase() === want) ||
      [...el.options].find(o => o.text.trim().toLowerCase().includes(want));
    if (!match) return { missing: true, options: [...el.options].slice(0, 20).map(o => cut(o.text, 40)) };
    el.value = match.value;
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return { kept: el.value === match.value, shown: cut(el.selectedOptions?.[0]?.text, 60) };
  };
  // Black boxes over every filled secret field on screen, for a screenshot.
  S.mask = on => {
    for (const m of S.masks) m.remove();
    S.masks = [];
    if (!on) return 0;
    for (const el of document.querySelectorAll('input')) {
      if (!secret(el) || !el.value) continue;
      const r = box(el);
      if (!r) continue;
      const m = document.createElement('div');
      m.style.cssText = `position:fixed;left:${r.left}px;top:${r.top}px;width:${r.width}px;` +
        `height:${r.height}px;background:#000;z-index:2147483647;pointer-events:none`;
      document.documentElement.appendChild(m);
      S.masks.push(m);
    }
    return S.masks.length;
  };
  // What the page shows as text, field values included (never a secret
  // field's), for checking that no typed secret is on screen.
  S.shown = () => {
    const values = [...document.querySelectorAll('input,textarea')]
      .filter(el => !secret(el) && el.value && box(el)).map(el => el.value);
    return [document.body?.innerText || '', ...values].join('\n');
  };
  // Resolves once the document has gone `quietMs` without a mutation, or
  // after `capMs`: an input's effect (a menu on a hover timer, a reply
  // redrawing a list) lands before the page is read again.
  S.quiet = (quietMs, capMs) => new Promise(resolve => {
    const start = performance.now();
    let last = start;
    const watch = new MutationObserver(() => { last = performance.now(); });
    watch.observe(document.documentElement,
      { subtree: true, childList: true, attributes: true, characterData: true });
    const tick = () => {
      const now = performance.now();
      if (now - last >= quietMs || now - start >= capMs) {
        watch.disconnect();
        resolve(Math.round(now - start));
      } else setTimeout(tick, 50);
    };
    setTimeout(tick, 50);
  });
  window.__roderDirect = S;
})()
