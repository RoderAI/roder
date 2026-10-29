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
  const clean = text => {
    const value = String(text || '').replace(/\s+/g, ' ').trim();
    return value.length <= 160 ? value : '[long text omitted]';
  };
  const roles = { MAIN: 'main', NAV: 'navigation', HEADER: 'banner', FOOTER: 'contentinfo',
    FORM: 'form', FIELDSET: 'group', DIALOG: 'dialog', UL: 'list', OL: 'list', LI: 'listitem',
    TABLE: 'table', THEAD: 'rowgroup', TBODY: 'rowgroup', TR: 'row', TH: 'columnheader', TD: 'cell' };
  const role = el => el.getAttribute('role') || (/^H[1-6]$/.test(el.tagName) ? 'heading' : roles[el.tagName]);
  const name = el => {
    const labelled = el.getAttribute('aria-labelledby');
    return clean(el.getAttribute('aria-label') || (labelled && labelled.split(/\s+/)
      .map(x => document.getElementById(x)?.innerText || '').join(' ')) ||
      (el.tagName === 'FIELDSET' ? el.querySelector('legend')?.innerText : '') ||
      (['heading', 'alert', 'status', 'columnheader', 'cell'].includes(role(el)) ? el.innerText : ''));
  };
  const visible = el => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden' && s.opacity !== '0';
  };
  const inView = el => {
    const r = el.getBoundingClientRect();
    return visible(el) && r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth;
  };
  const semantic = 'main,nav,header,footer,form,fieldset,dialog,ul,ol,li,h1,h2,h3,h4,h5,h6,' +
    '[role=heading],[role=alert],[role=status],[role=dialog],[role=alertdialog],[role=group],' +
    '[role=region],[role=list],[role=listitem],[role=listbox],[role=menu],[role=tablist],[role=tree],' +
    'table,thead,tbody,tr,th,td,[role=table],[role=rowgroup],[role=row],[role=columnheader],[role=cell]';
  S.snapshot = (page, opts) => {
    const selected = new Map();
    for (const item of page.elements) {
      const el = S.nodes.get(item.ref);
      if (el?.isConnected && (!item.offscreen || el === document.activeElement)) selected.set(el, item);
    }
    const nodes = new Set(selected.keys());
    let scanned = 0;
    // Include headings, status messages and group ancestors without inventing
    // clickable refs for passive structure. Keep DOM order, not screen position.
    const context = [...document.querySelectorAll(semantic)];
    for (const el of context) {
      if (++scanned > 2000) break;
      if (['heading', 'alert', 'status', 'columnheader', 'cell'].includes(role(el)) && inView(el)) nodes.add(el);
    }
    for (const el of [...nodes]) {
      for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
        if (p.matches(semantic) && visible(p)) nodes.add(p);
      }
    }
    const ordered = [...nodes].sort((a, b) => a === b ? 0 :
      a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    // The raw list prioritizes on-screen controls, so omitted off-screen ones
    // do not make this viewport snapshot incomplete. An omitted on-screen
    // control does: never infer removal from a capped observation.
    const onScreenCount = page.elements.filter(item => !item.offscreen).length;
    const controlsLimited = page.omitted > 0 && onScreenCount === page.elements.length;
    const limited = ordered.length > 100 || scanned > 2000 || controlsLimited;
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
        for (const key of ['ref', 'value', 'secret', 'filled', 'checked', 'selected', 'busy', 'expanded', 'disabled', 'draggable', 'options', 'offscreen']) {
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
    const current = { url: page.url, title: page.title, document: S.document,
      rows, limited, text:page.text, text_cut:page.text_cut, text_hash: hash, text_length: full.length };
    const before = opts.full ? null : previous;
    if (opts.publish) previous = current;
    return {...current, previous: before};
  };
})();
