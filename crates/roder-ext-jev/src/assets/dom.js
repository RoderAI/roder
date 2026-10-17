// A bounded semantic DOM tree, following the role/name/state shape of Codex's
// browser snapshots. Observation only: node refs identify existing controls;
// only the offered action space grants an action. Never read secret values.
(({tree,visible,safe,name,role,identity,actions,rectOf,placed}) => {
  const lines=[], seen=new Set(), offered=new Map(); let size=0, visited=0, omitted=false;
  for (const a of actions) if (a.node && !offered.has(a.node)) offered.set(a.node,a);
  const implicit={MAIN:'main',NAV:'navigation',ASIDE:'complementary',HEADER:'banner',
    FOOTER:'contentinfo',FORM:'form',DIALOG:'dialog',UL:'list',OL:'list',LI:'listitem',
    TABLE:'table',THEAD:'rowgroup',TBODY:'rowgroup',TR:'row',TH:'columnheader',TD:'cell',
    SECTION:'region',ARTICLE:'article',P:'paragraph',IMG:'img'};
  // Keep complete strings until the Rust secret scrubber runs. Truncating or
  // collapsing whitespace here can turn a known secret into an unmatchable echo.
  // The whole-line budget below still bounds the snapshot without partial labels.
  const quote=s=>JSON.stringify(String(s).toWellFormed());
  const add=(depth,line)=>{
    const text='  '.repeat(Math.min(depth,10))+'- '+line;
    if (lines.length>=100 || size+text.length+1>6000) { omitted=true; return false; }
    lines.push(text); size+=text.length+1; return true;
  };
  const onScreen=(e,r)=>{
    if (!r) return false;
    let left=Math.max(0,r.left), right=Math.min(innerWidth,r.right),
      top=Math.max(0,r.top), bottom=Math.min(innerHeight,r.bottom);
    const frameClip=tree.place(e.ownerDocument,placed)?.clip;
    if (frameClip) { left=Math.max(left,frameClip.left); right=Math.min(right,frameClip.right);
      top=Math.max(top,frameClip.top); bottom=Math.min(bottom,frameClip.bottom); }
    for (let p=tree.up(e); p; p=tree.up(p)) {
      const style=p.ownerDocument.defaultView.getComputedStyle(p), box=rectOf(p);
      if (!box) continue;
      if (['hidden','clip','auto','scroll'].includes(style.overflowX)) { left=Math.max(left,box.left); right=Math.min(right,box.right); }
      if (['hidden','clip','auto','scroll'].includes(style.overflowY)) { top=Math.max(top,box.top); bottom=Math.min(bottom,box.bottom); }
    }
    return right>left && bottom>top;
  };
  const headingText=e=>{
    const words=[], walk=e.ownerDocument.createTreeWalker(e,NodeFilter.SHOW_TEXT);
    for (let n=walk.nextNode(); n; n=walk.nextNode()) {
      let allowed=true;
      for (let p=n.parentElement; p; p=p.parentElement) {
        if (['INPUT','TEXTAREA','SELECT','SCRIPT','STYLE'].includes(p.tagName) || !visible(p)) { allowed=false; break; }
        if (p===e) break;
      }
      if (allowed) words.push(n.textContent);
    }
    return words.join(' ');
  };
  const walk=(e,depth,quiet=false)=>{
    if (!e || seen.has(e) || omitted) return;
    if (++visited>5000) { omitted=true; return; }
    seen.add(e);
    if (['SCRIPT','STYLE','NOSCRIPT','TEMPLATE'].includes(e.tagName) || !visible(e)) return;
    const box=rectOf(e), inView=onScreen(e,box);
    const heading=/^H[1-6]$/.test(e.tagName);
    const kind=role(e)||e.getAttribute('role')||(heading ? 'heading' : implicit[e.tagName]);
    const action=offered.get(identity(e));
    let childDepth=depth;
    if (kind && (inView || action)) {
      let label=(heading ? headingText(e) : name(e)) || action?.label;
      // Containers get only explicit names; their descendants supply content.
      if (!action && ['main','navigation','complementary','banner','contentinfo','form','dialog','list','listitem','table','rowgroup','row','columnheader','cell','region','article','paragraph','status','alert'].includes(kind))
        label=e.getAttribute('aria-label') || (e.getAttribute('aria-labelledby')||'').split(/\s+/).map(id=>name(e.getRootNode().getElementById?.(id) || e.ownerDocument.getElementById(id))).join(' ');
      const states=[];
      if (action) states.push('node='+action.node);
      if (heading) states.push('level='+e.tagName.slice(1));
      if (e.matches(':disabled') || e.getAttribute('aria-disabled')==='true') states.push('disabled');
      if (e===tree.active()) states.push('active');
      const checked=action?.checked ??
        (e.tagName==='INPUT' && ['checkbox','radio'].includes(e.type) ? e.checked : null);
      for (const attr of ['checked','selected','expanded','busy']) {
        if (attr==='checked' && checked!==null) continue;
        const v=e.getAttribute('aria-'+attr);
        if (v!==null) states.push(attr+'='+v);
      }
      if (checked!==null) states.push('checked='+checked);
      if (action?.offscreen) states.push('offscreen');
      if (safe(e) && action?.kind==='fill' && action.value) states.push('value='+quote(action.value));
      if (action?.kind==='select') states.push('value='+quote(action.current_value||''));
      if (action?.filled && !safe(e)) states.push('filled');
      if (!add(depth,kind+(label ? ' '+quote(label) : '')+states.map(s=>' ['+s+']').join(''))) return;
      childDepth=depth+1;
    }
    // Never descend into a field's initial DOM value (notably textarea).
    // Headings may contain offered links/buttons; retain them while suppressing
    // duplicate text already represented by the heading's accessible name.
    if (['INPUT','TEXTAREA','SELECT','BUTTON','A'].includes(e.tagName) || e.isContentEditable) return;
    const frame=tree.frameDoc(e);
    if (frame?.body) { walk(frame.body,childDepth); return; }
    const children=e.shadowRoot ? e.shadowRoot.childNodes :
      e.tagName==='SLOT' && e.assignedNodes().length ? e.assignedNodes() : e.childNodes;
    for (const n of children) {
      if (n.nodeType===1) walk(n,childDepth,quiet||heading);
      else if (n.nodeType===3 && inView && !quiet && !heading && n.textContent.trim()) add(childDepth,'text '+quote(n.textContent));
      if (omitted) break;
    }
  };
  walk(document.body,0);
  if (omitted) lines.push('- … (DOM snapshot truncated)');
  return lines.join('\n');
})
