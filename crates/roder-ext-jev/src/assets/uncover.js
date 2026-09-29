// Jev's own. When act.js finds an observed target covered, this looks at
// what covers it: a popover, menu, picker or dialog laid over the page (an
// element positioned fixed, absolute or sticky, a top-layer dialog or
// popover, or one with a dialog, menu, listbox or tooltip role), which Jev
// may have opened itself. It returns how the page may let it be dismissed,
// for page/uncover.rs to try in order: Escape; the layer's own close
// control (a button whose whole name is a close word, never an accept,
// cancel or done); a press outside it on nothing that acts (outside a small
// layer, or on a full-page backdrop). Null when nothing covers the target,
// or what covers it is not such a layer. A cookie or consent banner is left
// to banner refusal ({consent:true}), and a layer that did not go before is
// not tried again ({stuck:true}); phase 'failed' records the last one so.
// Called with {node, phase}.
(a => {
  const c=window.__jevFast, e=c?.nodes.get(a.node), tree=c?.tree;
  if (!tree || !e?.isConnected) return null;
  if (a.phase==='failed') {
    if (c.lastLayer) (c.stuck ||= new WeakSet()).add(c.lastLayer);
    return true;
  }
  const r=tree.rect(e);
  if (!r?.width || !r.height) return null;
  const vw=innerWidth, vh=innerHeight, area=b=>b ? Math.max(0,b.width)*Math.max(0,b.height) : 0;
  const clamp=(v,lo,hi)=>Math.min(Math.max(v,lo),hi);
  // What takes a press at the target's points.
  let cover=null;
  for (const [fx,fy] of [[.5,.5],[.25,.25],[.75,.25],[.25,.75],[.75,.75]]) {
    const x=clamp(r.left+r.width*fx,0,vw-1), y=clamp(r.top+r.height*fy,0,vh-1);
    const h=tree.hit(x,y);
    if (h && !tree.within(h,e) && !tree.within(e,h)) { cover=h; break; }
  }
  if (!cover) return null;
  const style=x=>x.ownerDocument.defaultView.getComputedStyle(x);
  const LAYER_ROLES=['dialog','alertdialog','menu','listbox','tooltip','grid'];
  const layered=x=>['fixed','absolute','sticky'].includes(style(x).position) ||
    (x.tagName==='DIALOG' && x.open) || (x.matches?.(':popover-open')) ||
    LAYER_ROLES.includes(x.getAttribute?.('role')) || x.getAttribute?.('aria-modal')==='true';
  // The largest branch that holds what covers the target but not the target.
  let layer=cover, overlay=layered(cover);
  for (let x=tree.up(cover); x && !tree.within(e,x) && x!==x.ownerDocument.body &&
       x!==x.ownerDocument.documentElement; x=tree.up(x)) {
    layer=x;
    overlay ||= layered(x);
  }
  if (!overlay) return null;
  const words=(layer.innerText||'').slice(0,2000);
  if (/cookie|consent|gdpr/i.test(words)) return {consent:true};
  if (c.stuck?.has(layer)) return {stuck:true};
  c.lastLayer=layer;
  const name=x=>(x.getAttribute('aria-label')||x.getAttribute('title')||x.innerText||x.value||'')
    .replace(/\s+/g,' ').trim();
  const say=x=>name(x).slice(0,60) || x.tagName.toLowerCase();
  const out={cover:say(cover)};
  // A close control of the layer's own: its whole name a close word, or an
  // aria-label or title saying close or dismiss.
  const CLOSE=/^(close|close (dialog|menu|popup|window|calendar|picker)|dismiss|hide|no thanks|not now|maybe later|[×✕✖╳xX])$/i;
  const controls='button,[role="button"],a:not([href]),[aria-label],[title]';
  for (const x of layer.querySelectorAll(controls)) {
    const label=name(x);
    const said=x.getAttribute('aria-label')||x.getAttribute('title')||'';
    if (!CLOSE.test(label) && !/\b(close|dismiss)\b/i.test(said)) continue;
    const b=tree.rect(x);
    if (!b?.width || !b.height || !c.visible(x)) continue;
    const p={x:b.left+b.width/2, y:b.top+b.height/2};
    if (p.x<0 || p.y<0 || p.x>=vw || p.y>=vh) continue;
    const h=tree.hit(p.x,p.y);
    if (h && tree.within(h,x)) { out.close={...p, label:label.slice(0,60)}; break; }
  }
  // A press on nothing that acts: not a control, a generic clickable or a
  // pointer region, and not the target.
  const acting='a[href],button,input,textarea,select,summary,label,[contenteditable="true"],[onclick],'+
    '[role="button"],[role="link"],[role="option"],[role="menuitem"],[role="tab"],[role="checkbox"],'+
    '[role="radio"],[role="switch"],[role="gridcell"],iframe';
  const inert=h=>{
    for (let x=h; x; x=tree.up(x)) {
      if (x===e || x.matches?.(acting) || c.generic?.has(x)) return false;
      if (style(x).cursor==='pointer' && x!==layer) return false;
    }
    return true;
  };
  const lb=tree.rect(layer);
  const big=area(lb)>=0.6*vw*vh;
  for (let i=1; i<8 && !out.outside; i++) {
    for (let j=1; j<8; j++) {
      const x=vw*j/8, y=vh*i/8;
      if (x>=r.left-4 && x<=r.right+4 && y>=r.top-4 && y<=r.bottom+4) continue;
      const h=tree.hit(x,y);
      if (!h || !inert(h)) continue;
      // Outside a small layer; on a large one only its bare backdrop (an
      // element that covers most of the page and holds no control).
      const inside=tree.within(h,layer);
      if (!big && inside) continue;
      if (!big && lb && x>=lb.left && x<=lb.right && y>=lb.top && y<=lb.bottom) continue;
      if (big && (!inside || h.querySelector?.(acting) || area(tree.rect(h))<0.6*vw*vh)) continue;
      out.outside={x,y};
      break;
    }
  }
  return out;
})
