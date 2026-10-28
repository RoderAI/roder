// Semantic observations for direct tools. State lives in this isolated document,
// so reconnecting a DirectSession does not discard the last published snapshot.
// Private outcome probes never publish; explicit looks always recover a full tree.
(() => {
  const S = window.__roderDirect;
  if (S.snapshot) return;
  const ids = new WeakMap();
  let next = 1, previous = null;
  const id = el => {
    if (!ids.has(el)) ids.set(el, `s${next++}`);
    return ids.get(el);
  };
  const clean = (text, max = 100) => String(text || '').replace(/\s+/g, ' ').trim().slice(0, max);
  const roles = { MAIN: 'main', NAV: 'navigation', HEADER: 'banner', FOOTER: 'contentinfo',
    FORM: 'form', FIELDSET: 'group', DIALOG: 'dialog', UL: 'list', OL: 'list', LI: 'listitem' };
  const role = el => el.getAttribute('role') || (/^H[1-6]$/.test(el.tagName) ? 'heading' : roles[el.tagName]);
  const name = el => {
    const labelled = el.getAttribute('aria-labelledby');
    return clean(el.getAttribute('aria-label') || (labelled && labelled.split(/\s+/)
      .map(x => document.getElementById(x)?.innerText || '').join(' ')) ||
      (el.tagName === 'FIELDSET' ? el.querySelector('legend')?.innerText : '') ||
      (['heading', 'alert', 'status'].includes(role(el)) ? el.innerText : ''));
  };
  const visible = el => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden' && s.opacity !== '0';
  };
  const semantic = 'main,nav,header,footer,form,fieldset,dialog,ul,ol,li,h1,h2,h3,h4,h5,h6,' +
    '[role=heading],[role=alert],[role=status],[role=dialog],[role=alertdialog],[role=group],' +
    '[role=region],[role=list],[role=listitem],[role=listbox],[role=menu],[role=tablist],[role=tree]';
  S.snapshot = (page, opts) => {
    const selected = new Map();
    for (const item of page.elements) {
      const el = S.nodes.get(item.ref);
      if (el?.isConnected) selected.set(el, item);
    }
    const nodes = new Set(selected.keys());
    let scanned = 0;
    // Include headings, status messages and group ancestors without inventing
    // clickable refs for passive structure. Keep DOM order, not screen position.
    const context = [...document.querySelectorAll(semantic)];
    for (const el of context) {
      if (++scanned > 2000) break;
      if (['heading', 'alert', 'status'].includes(role(el)) && visible(el)) nodes.add(el);
    }
    for (const el of [...nodes]) {
      for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
        if (p.matches(semantic) && visible(p)) nodes.add(p);
      }
    }
    const ordered = [...nodes].sort((a, b) => a === b ? 0 :
      a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    const limited = ordered.length > 100 || scanned > 2000 || page.omitted > 0;
    const rows = ordered.slice(0, 100).map(el => {
      const item = selected.get(el);
      let p = el.parentElement;
      while (p && !nodes.has(p)) p = p.parentElement;
      const row = { id: id(el), parent: p ? id(p) : null,
        role: item?.role || (el.tagName === 'INPUT' ?
          ({ checkbox: 'checkbox', radio: 'radio', range: 'slider', number: 'spinbutton',
            password: 'password', button: 'button', submit: 'button', reset: 'button' }[item?.type] || 'textbox') : null) || role(el) ||
          ({ A: 'link', SELECT: 'combobox', TEXTAREA: 'textbox' }[el.tagName]) || el.tagName.toLowerCase(),
        label: item ? item.label : name(el) };
      if (item) {
        for (const key of ['ref', 'value', 'secret', 'filled', 'checked', 'expanded', 'disabled', 'draggable', 'options', 'offscreen']) {
          if (item[key] !== undefined) row[key] = item[key];
        }
      }
      if (el === document.activeElement) row.focused = true;
      if (row.role === 'heading') row.level = Number(el.getAttribute('aria-level') || el.tagName.slice(1)) || undefined;
      return row;
    });
    // Hash the same complete text for both brief/full reads; excerpt lengths
    // must never manufacture a text change. Store a hash, not duplicate text.
    const full = document.body?.innerText || '';
    let hash = 2166136261;
    for (let i = 0; i < full.length; i++) hash = Math.imul(hash ^ full.charCodeAt(i), 16777619);
    const current = { url: page.url, title: page.title, rows, limited, hash, textLength: full.length };
    let fullMode = opts.full || !previous || previous.url !== current.url || limited || previous.limited;
    const old = new Map((previous?.rows || []).map(row => [row.id, row]));
    const now = new Map(rows.map(row => [row.id, row]));
    const changes = rows.filter(row => JSON.stringify(old.get(row.id)) !== JSON.stringify(row))
      .map(row => ({ ...row, change: old.has(row.id) ? 'changed' : 'added' }));
    const removed = (previous?.rows || []).filter(row => !now.has(row.id));
    if (changes.length + removed.length > 100) fullMode = true;
    const textChanged = !previous || previous.hash !== hash || previous.textLength !== full.length;
    const snapshot = { mode: fullMode ? 'full' : 'diff', rows: fullMode ? rows : changes,
      removed: fullMode ? [] : removed, context: rows.filter(row => changes.some(c => c.parent === row.id)),
      limited, text_changed: fullMode || textChanged,
      title_changed: !previous || previous.title !== current.title };
    if (opts.publish) previous = current;
    return snapshot;
  };
})();
