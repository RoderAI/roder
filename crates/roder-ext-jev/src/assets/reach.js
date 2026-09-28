// Whether a control snapshot.js found can be reached (Jev's own). Called
// with snapshot.js's `tree` and `rectOf`. A control above or below the
// viewport is kept only if scrolling can bring it into view: the document's
// scrolling, or that of a box it sits in (an app shell's main pane, where
// the document itself does not scroll). Not one above the top or past the
// end of what scrolls (a skip link parked at top:-40px), nor one inside a
// fixed layer (a banner translated off the top). A control an ancestor cuts
// off where nothing can scroll it (a collapsed panel, overflow: clip) is not
// reached either.
(({tree, rectOf}) => {
  const pinned=new Map();
  const fixed=e=>{
    const path=[]; let result=false;
    for (let x=e; x && x!==document.documentElement; x=tree.up(x)) {
      if (pinned.has(x)) { result=pinned.get(x); break; }
      path.push(x);
      if (x.nodeType===1 && getComputedStyle(x).position==='fixed') { result=true; break; }
    }
    for (const x of path) pinned.set(x,result);
    return result;
  };
  // The element above x in its own document: its parent, or a shadow
  // root's host.
  const up=x=>x.parentElement || x.parentNode?.host || null;
  const docHeight=document.documentElement.scrollHeight;
  const inDocument=r=>r.bottom+scrollY>0 && r.top+scrollY<docHeight;
  // x itself when it scrolls on its own (overflow auto or scroll, with more
  // than it shows), else the nearest ancestor that does, within x's
  // document; null for none. Memoized along the path.
  const held=new Map();
  const scrolls=x=>x.scrollHeight>x.clientHeight+1 &&
    ['auto','scroll'].includes(getComputedStyle(x).overflowY);
  const holder=start=>{
    const path=[]; let found=null;
    for (let x=start; x && x!==x.ownerDocument.documentElement; x=up(x)) {
      if (held.has(x)) { found=held.get(x); break; }
      path.push(x);
      if (scrolls(x)) { found=x; break; }
    }
    for (const x of path) held.set(x,found);
    return found;
  };
  // Inside the content of the box that scrolls it, and that box on screen
  // or itself in reach.
  const inBox=(e,r)=>{
    const box=holder(up(e)), b=box && rectOf(box);
    if (!b) return false;
    const top=r.top-b.top+box.scrollTop;
    if (top+r.height<=0 || top>=box.scrollHeight) return false;
    return (b.bottom>0 && b.top<innerHeight) || reachable(box,b);
  };
  const reachable=(e,r)=>inDocument(r) || inBox(e,r);
  // A hidden-overflow box with room, such as a carousel, still scrolls for
  // act.js, so it is kept; a collapsed panel (height:0; overflow:hidden) or
  // overflow:clip cuts off what it holds.
  const boxes=new Map();
  const box=x=>{
    if (!boxes.has(x)) {
      const style=getComputedStyle(x), r=rectOf(x);
      // A display:contents box has no size and clips nothing.
      const cut=(flow,size)=>style.display!=='contents' &&
        (flow==='clip' || (flow==='hidden' && size<1));
      boxes.set(x,{position:style.position,r,
        x:cut(style.overflowX,x.clientWidth),y:cut(style.overflowY,x.clientHeight)});
    }
    return boxes.get(x);
  };
  const clipped=(e,r)=>{
    // An absolute box is clipped only from its containing block upwards.
    // Within its own document: its frame's box clips it apart.
    let free=getComputedStyle(e).position;
    for (let x=up(e); x && x!==x.ownerDocument.documentElement; x=up(x)) {
      if (free==='fixed') return false;
      const b=box(x);
      if (free==='absolute' && b.position==='static') continue;
      free=b.position;
      if (b.x && (r.right<=b.r.left || r.left>=b.r.right)) return true;
      if (b.y && (r.bottom<=b.r.top || r.top>=b.r.bottom)) return true;
    }
    return false;
  };
  return {fixed,reachable,clipped};
})
