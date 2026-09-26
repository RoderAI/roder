// Vendored from upstream Jev's browser.py (MIT, browser-use/jev-ultrafast).
// After input, let the page react before it is read again: a couple of frames,
// or a short window for an autocomplete to populate its listbox.
(action => new Promise(resolve => {
  const field=window.__jevFast?.nodes.get(action.node);
  const autocomplete=action.kind==='fill' && field?.getAttribute('role')==='combobox';
  let frames=0, stopped=false;
  const finish=()=>{stopped=true;resolve()};
  setTimeout(finish,autocomplete ? 200 : 50);
  const ready=()=>{
    if (stopped) return;
    const ids=(field?.getAttribute('aria-controls')||field?.getAttribute('aria-owns')||'')
      .split(/\s+/).filter(Boolean);
    const roots=ids.length ? ids.map(id=>document.getElementById(id)).filter(Boolean) : [document];
    if (autocomplete && roots.some(root=>root.querySelector('[role="option"],[role="listbox"] li'))) return finish();
    if (++frames>=2 && !autocomplete) return finish();
    requestAnimationFrame(ready);
  };
  requestAnimationFrame(ready);
}))