// Jev's quiet clock (jev-owned; the idea follows fastbrowse's snapshot.js,
// MIT). Installed on every new document and again before each read, so it
// must be idempotent, and it creates the whole `__jevFast` shape because
// snapshot.js only fills that object in when it is missing.
(() => {
  const state = window.__jevFast ||= {ids:new WeakMap(), nodes:new Map(), next:1};
  if (state.lastMutation !== undefined) return;
  const changed = () => { state.lastMutation = performance.now(); };
  changed();
  const options={subtree:true, childList:true, characterData:true, attributes:true};
  const observer=new MutationObserver(changed);
  // An input handler can defer a redraw or a navigation without mutating
  // yet, so the clock also restarts at the input itself.
  const inputs=['input','change','scroll','wheel','pointermove','pointerdown','pointerup',
    'keydown','keyup'];
  // snapshot.js hands over every open shadow root and same-origin frame it
  // walks, so changes there restart the clock too (from the first read on).
  const watched=new WeakSet();
  state.watch=root=>{
    if (watched.has(root)) return;
    watched.add(root);
    observer.observe(root, options);
    if (root.nodeType===9) for (const event of inputs) root.addEventListener(event, changed, true);
  };
  state.watch(document);
  // What a field's suggestion lists show: the visible options of the lists
  // it controls or owns, and of any listbox. fill.js keeps it from before
  // the text is typed, so settle.js can tell the options for the new text
  // from a list still showing the last query's (a debounced update).
  state.suggestions=field=>{
    const ids=(field?.getAttribute('aria-controls')||field?.getAttribute('aria-owns')||'')
      .split(/\s+/).filter(Boolean);
    const lists=ids.map(id=>field.getRootNode().getElementById?.(id) || document.getElementById(id))
      .filter(Boolean);
    const options=new Set([...lists.flatMap(list=>[...list.querySelectorAll('[role="option"],li,td')]),
      ...document.querySelectorAll('[role="listbox"] [role="option"]')]);
    return [...options].filter(o=>o.checkVisibility({checkOpacity:true,checkVisibilityCSS:true}))
      .map(o=>o.textContent.trim()).join('\n');
  };
})();
