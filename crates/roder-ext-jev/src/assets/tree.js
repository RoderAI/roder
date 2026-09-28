// The composed tree (Jev's own): open shadow roots and same-origin frames,
// placed in the top window's coordinates. snapshot.js walks with it and
// keeps it as `__jevFast.tree`, so act.js and fill.js hit-test, measure and
// follow focus the same way. A frame's document is placed by its element's
// content box, scaled by width / offsetWidth for a transformed frame, and is
// clipped to that box. An element with `display: contents` is measured by
// what it shows (its children). Cross-origin frames (out of process) are not
// reachable from the page and are left out. The frame mapping and the
// shadow descent follow fastbrowse's browser/snapshot.js and page.py (MIT).
(() => {
  const frameOf=doc=>{ try { return doc?.defaultView?.frameElement || null; } catch { return null; } };
  const frameDoc=e=>{
    if (e?.tagName!=='IFRAME' && e?.tagName!=='FRAME') return null;
    try { return e.contentDocument; } catch { return null; }
  };
  // The element above x: its parent, a shadow root's host, or the element of
  // the frame a document sits in.
  const up=x=>x.parentElement || x.parentNode?.host ||
    (x.parentNode?.nodeType===9 ? frameOf(x.parentNode) : null);
  const within=(x,e)=>{ for (; x; x=up(x)) if (x===e) return true; return false; };
  const TOP={x:0,y:0,s:1,clip:null};
  const cut=(a,b)=>!b ? a : {left:Math.max(a.left,b.left),top:Math.max(a.top,b.top),
    right:Math.min(a.right,b.right),bottom:Math.min(a.bottom,b.bottom)};
  // Where a document's viewport sits in the top window: {x, y, s, clip}, or
  // null for one that is not a same-origin frame of it.
  const place=(doc,memo)=>{
    if (doc===document) return TOP;
    if (memo?.has(doc)) return memo.get(doc);
    const f=frameOf(doc), parent=f && place(f.ownerDocument,memo);
    let found=null;
    if (parent) {
      const r=f.getBoundingClientRect(), scale=f.offsetWidth ? r.width/f.offsetWidth : 1;
      const style=f.ownerDocument.defaultView.getComputedStyle(f);
      const left=r.left+(f.clientLeft+parseFloat(style.paddingLeft))*scale;
      const top=r.top+(f.clientTop+parseFloat(style.paddingTop))*scale;
      const x=parent.x+left*parent.s, y=parent.y+top*parent.s, s=parent.s*scale;
      const view=doc.defaultView;
      found={x,y,s,clip:cut({left:x,top:y,right:x+view.innerWidth*s,bottom:y+view.innerHeight*s},parent.clip)};
    }
    memo?.set(doc,found);
    return found;
  };
  const map=(r,p)=>{
    const left=p.x+r.left*p.s, top=p.y+r.top*p.s, width=r.width*p.s, height=r.height*p.s;
    return {left,top,right:left+width,bottom:top+height,x:left,y:top,width,height};
  };
  // An element's own boxes. One with `display: contents` (a link wrapping
  // a whole card) has none of its own: what it shows is its children's
  // boxes and its text's.
  const contents=e=>e.ownerDocument.defaultView.getComputedStyle(e).display==='contents';
  const boxes=e=>{
    const own=e.getClientRects();
    if (own.length || !contents(e)) return [...own];
    const found=[];
    for (const n of e.childNodes) {
      if (n.nodeType===1) found.push(...boxes(n));
      else if (n.nodeType===3 && n.textContent.trim()) {
        const range=e.ownerDocument.createRange();
        range.selectNodeContents(n);
        found.push(...range.getClientRects());
      }
    }
    return found;
  };
  const union=list=>{
    const shown=list.filter(r=>r.width>0 && r.height>0);
    if (!shown.length) return null;
    const left=Math.min(...shown.map(r=>r.left)), top=Math.min(...shown.map(r=>r.top));
    const right=Math.max(...shown.map(r=>r.right)), bottom=Math.max(...shown.map(r=>r.bottom));
    return {left,top,right,bottom,x:left,y:top,width:right-left,height:bottom-top};
  };
  // Its box in its own document: for display:contents, around its
  // children's.
  const local=e=>{
    const r=e.getBoundingClientRect();
    return r.width || r.height || !contents(e) ? r : union(boxes(e)) || r;
  };
  // An element's box in the top window, or null when its frame is gone.
  const rect=(e,memo)=>{
    const r=local(e);
    if (e.ownerDocument===document) return r;
    const p=place(e.ownerDocument,memo);
    return p ? map(r,p) : null;
  };
  const rects=e=>{
    const p=place(e.ownerDocument);
    return !p ? [] : boxes(e).map(r=>p===TOP ? r : map(r,p));
  };
  // The deepest element at a top-window point, through shadow roots and
  // same-origin frames.
  const hit=(x,y)=>{
    let at=document.elementFromPoint(x,y), p=TOP;
    for (let depth=0; at && depth<32; depth++) {
      const local=[(x-p.x)/p.s,(y-p.y)/p.s];
      const inner=at.shadowRoot?.elementFromPoint(...local);
      if (inner && inner!==at) { at=inner; continue; }
      const doc=frameDoc(at), q=doc && place(doc);
      const framed=q && doc.elementFromPoint((x-q.x)/q.s,(y-q.y)/q.s);
      if (framed) { at=framed; p=q; continue; }
      break;
    }
    return at;
  };
  // The focused element, inside shadow roots and same-origin frames.
  const active=()=>{
    let a=document.activeElement;
    for (let depth=0; a && depth<32; depth++) {
      const inner=a.shadowRoot?.activeElement || frameDoc(a)?.activeElement;
      if (!inner || inner===a || inner===frameDoc(a)?.body) break;
      a=inner;
    }
    return a;
  };
  return {frameOf,frameDoc,up,within,place,map,rect,rects,hit,active,contents,TOP};
})()
