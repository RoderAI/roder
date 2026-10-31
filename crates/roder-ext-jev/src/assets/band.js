// Reaching a target under a band fixed to the window (Jev's own; the rules
// follow fastbrowse's page/act.rs route_around and keys_reach, MIT). A band
// (an anchored ad, a sticky footer, a chat bubble) covers a place on the
// screen, not a control, so the target is scrolled clear of it; where the
// page cannot scroll it clear, the target is pressed from the keyboard, as a
// keyboard user would, and nothing in the band is ever touched. A dialog, an
// open menu or popover, a cover the size of half the window and a panel in
// the page's own flow are not bands: the page shows something else there,
// and that is page/uncover.rs's to deal with or the run's to report.
({
  // What covers the target: {kind, x, y}, the kind 'none', 'obstruction' (a
  // dialog, menu, listbox or popover the page means to be dealt with first),
  // 'consent' (a cookie or consent bar), 'band' or 'other', the point in the
  // top window. The centre and the four quarter points are tried, as act.js
  // tries them: a point that is the target's own (its inside, its label, a
  // label's text) is 'none' at that point, so a field's own decoration over
  // its centre is no cover; otherwise what took the centre is classified.
  // Null when the target is gone or has no box.
  kind: ({node}) => {
    const c=window.__jevFast, e=c?.nodes.get(node), tree=c?.tree;
    if (!tree || !e?.isConnected) return null;
    const r=tree.rect(e);
    if (!r?.width || !r.height) return null;
    const vw=innerWidth, vh=innerHeight;
    const x=r.left+r.width/2, y=r.top+r.height/2;
    const at=kind=>({kind, x, y});
    // Out of view nothing is drawn over it: a key goes to it as it would.
    if (x<0 || y<0 || x>=vw || y>=vh) return at('none');
    const mine=h=>!h || tree.within(h,e) || tree.within(e,h) || h===e.control ||
      (h.nodeType===1 && h.closest('label')?.control===e);
    let hit=null;
    for (const [fx,fy] of [[.5,.5],[.25,.25],[.75,.25],[.25,.75],[.75,.75]]) {
      const px=r.left+r.width*fx, py=r.top+r.height*fy;
      if (px<0 || py<0 || px>=vw || py>=vh) continue;
      const h=tree.hit(px,py);
      if (mine(h)) return {kind:'none', x:px, y:py};
      hit ||= h;
    }
    const OBSTRUCTIONS='dialog,[role="dialog"],[role="alertdialog"],[aria-modal="true"],[role="menu"],'+
      '[role="listbox"],[popover]';
    for (let n=hit; n; n=tree.up(n)) {
      if (n.nodeType===1 && n.matches(OBSTRUCTIONS) && !tree.within(e,n)) return at('obstruction');
    }
    // A band is fixed or sticky, smaller than half the window, and anchored
    // to an edge of it: a box floating in the middle is a layer, not a band.
    const EDGE=48;
    let band=false;
    for (let n=hit; n && !tree.within(e,n); n=tree.up(n)) {
      if (n.nodeType!==1) continue;
      const style=n.ownerDocument.defaultView.getComputedStyle(n);
      if (!['fixed','sticky'].includes(style.position)) continue;
      const b=tree.rect(n);
      if (!b || b.width*b.height>=vw*vh/2) return at('other');
      if (b.top>EDGE && b.bottom<vh-EDGE && b.left>EDGE && b.right<vw-EDGE) return at('other');
      // A cookie or consent bar asks a question first, as a dialog does: it
      // is left to banner refusal, never got around (uncover.js reads it so).
      if (/cookie|consent|gdpr/i.test((n.innerText||'').slice(0,2000))) return at('consent');
      band=true;
    }
    return at(band ? 'band' : 'other');
  },
  // Scroll the target to `block` ('center', 'start' or 'end') of what
  // scrolls it. Whether it is still there.
  align: ({node, block}) => {
    const e=window.__jevFast?.nodes.get(node);
    if (!e?.isConnected) return false;
    e.scrollIntoView({block, inline:'nearest', behavior:'instant'});
    return true;
  },
  // The key the target's own semantics answer to: Space for a checkbox,
  // radio or switch, Enter for a button, link, summary, tab, menu item,
  // option or tree item; null for anything else, which is not pressed.
  key: ({node}) => {
    const e=window.__jevFast?.nodes.get(node);
    if (!e?.isConnected) return null;
    const role=e.getAttribute('role');
    if (e.tagName==='INPUT' && ['checkbox','radio'].includes(e.type)) return ' ';
    if (['checkbox','radio','switch','menuitemcheckbox','menuitemradio'].includes(role)) return ' ';
    if (e.matches('button,a[href],summary,input[type="submit"],input[type="button"],'+
      'input[type="reset"],input[type="image"]')) return 'Enter';
    if (['button','link','tab','menuitem','option','treeitem'].includes(role)) return 'Enter';
    return null;
  },
})
