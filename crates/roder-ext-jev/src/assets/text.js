// The page text snapshot.js reports (started from upstream Jev's
// snapshot.js, MIT; the rest is Jev's own). Called with snapshot.js's
// helpers; returns a function that reads the text shown in the viewport,
// 6000 characters at most, in the order a person reads it: a shadow host's
// shadow tree in place of its children, a slot's nodes where the slot is,
// and a same-origin frame's document where the frame is. What a text field
// holds is page text too, where the field is (a textarea's passage to copy,
// a filled-in form), never a secret's (see `safe`). Text a box has scrolled
// out of its own view is not shown, so scrolling the box shows something new.
// The function returns {text, held}: `held` lists the field values `text`
// holds whole, in order, so a caller can tell the page's own words from what
// a field holds (a completion check must not count a value typed in).
(({tree, visible, safe, placed, roots, regions, rectOf, cut}) => {
  // The scroll box, of those observed, that holds an element, and its rect.
  const holder=new Map(), rects=new Map(regions.map(e=>[e,rectOf(e)]));
  const boxOf=x=>{
    const path=[]; let found=null;
    for (let p=x; p && p!==document.body; p=p.parentElement) {
      if (holder.has(p)) { found=holder.get(p); break; }
      path.push(p);
      if (rects.has(p)) { found=rects.get(p); break; }
    }
    for (const p of path) holder.set(p,found);
    return found;
  };
  // A textarea's own child text is only its initial value.
  const typed=e=>e.tagName==='TEXTAREA' || (e.tagName==='INPUT' && safe(e) &&
    !['button','checkbox','color','image','radio','range','reset','submit'].includes(e.type));
  const shown=(r,clip)=>r.width>0 && r.height>0 && r.bottom>0 && r.top<innerHeight && r.right>0 &&
    r.left<innerWidth && (!clip || (r.bottom>clip.top && r.top<clip.bottom && r.right>clip.left &&
      r.left<clip.right));
  const words=[], fieldAt=new Set(); let length=0;
  const add=(value,field)=>{
    if (field) fieldAt.add(words.length);
    words.push(value); length+=value.length;
  };
  // Everything under `start` (a document, a shadow root, or an element a
  // slot shows), the element itself first.
  const read=start=>{
    const doc=start.nodeType===9 ? start : start.ownerDocument;
    const top=start.nodeType===9 ? start.body : start, where=tree.place(doc,placed);
    if (!top || !where) return;
    const walker=doc.createTreeWalker(top,NodeFilter.SHOW_TEXT|NodeFilter.SHOW_ELEMENT);
    const range=doc.createRange(), at=r=>where===tree.TOP ? r : tree.map(r,where);
    // Past the current node's children: its next sibling, or an ancestor's.
    const past=()=>{
      while (walker.currentNode!==top) {
        const next=walker.nextSibling();
        if (next) return next;
        if (!walker.parentNode()) return null;
      }
      return null;
    };
    const text=node=>{
      const value=node.textContent.trim();
      // A text node right in a shadow root sits in its host.
      const parent=node.parentElement || node.parentNode?.host;
      if (!value || !parent || parent.tagName==='TEXTAREA' ||
        parent.closest('script,style,noscript,template') || !visible(parent)) return;
      range.selectNodeContents(node); const r=at(range.getBoundingClientRect());
      const box=boxOf(parent);
      if (box && (r.bottom<=box.top || r.top>=box.bottom)) return;
      if (shown(r,where.clip)) add(value);
    };
    let node=top.nodeType===1 ? top : walker.nextNode();
    while (node && length<6000) {
      let inside=true;
      if (node.nodeType===3) text(node);
      else if (node.shadowRoot) {
        // Its children show only through the shadow tree's slots.
        read(node.shadowRoot);
        inside=false;
      } else if (node.tagName==='SLOT' && node.assignedNodes().length) {
        for (const n of node.assignedNodes()) n.nodeType===3 ? text(n) : n.nodeType===1 && read(n);
        inside=false;
      } else {
        const framed=tree.frameDoc(node);
        if (framed && roots.includes(framed)) read(framed);
        const value=typed(node) ? node.value.trim() : '';
        if (value && visible(node) && shown(at(node.getBoundingClientRect()),where.clip)) add(value,true);
      }
      node=inside ? walker.nextNode() : past();
    }
  };
  return ()=>{
    read(document);
    const text=cut(words.join('\n'),6000).toWellFormed();
    // Words are joined by one line break each; a field value cut by the
    // limit is not reported.
    const held=[]; let end=-1;
    words.forEach((word,i)=>{
      end+=word.length+1;
      if (fieldAt.has(i) && end<=text.length) held.push(word.toWellFormed());
    });
    return {text,held};
  };
})
