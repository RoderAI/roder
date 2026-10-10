// Started from upstream Jev's snapshot.js (MIT, browser-use/jev-ultrafast);
// now Jev-owned. Called with tree.js's helpers and the observation's parts
// (see page.rs): names.js names what is offered, reach.js says whether an
// offscreen control can be reached, and text.js reads the page text. Open
// shadow roots and same-origin frames are observed like the document itself.
((tree, parts) => {
  if (!document.body) return null;
  const cache = window.__jevFast ||= {ids:new WeakMap(), nodes:new Map(), next:1};
  // One placement per frame for this snapshot; boxes are in the top window.
  const placed=new Map(), rectOf=e=>tree.rect(e,placed);
  const identity = e => {
    if (!cache.ids.has(e)) cache.ids.set(e,cache.next++);
    const id=cache.ids.get(e); cache.nodes.set(id,e); return id;
  };
  // A node of a frame's old document is still connected to that document
  // after the frame navigates; it has no window any more.
  for (const [id,e] of cache.nodes) if (!e.isConnected || !e.ownerDocument.defaultView) cache.nodes.delete(id);
  // A field whose value is a secret: a password, one masked with
  // -webkit-text-security, or one marked for a password or a one-time code.
  // It is offered to type into, but what it holds is never read: the
  // observation says only whether it is filled, and its value, guard and
  // page text leave it out. A field once seen as a secret stays one (a
  // "show password" toggle turns it into a plain text input).
  const secrets=cache.secrets ||= new WeakSet();
  const secretNow = e => e.type==='password' ||
    /(?:password|one-time-code)$/i.test(e.getAttribute('autocomplete')||'') ||
    ((e.tagName==='INPUT' || e.tagName==='TEXTAREA') &&
      !['none',''].includes(getComputedStyle(e).webkitTextSecurity||''));
  const secret = e => secrets.has(e) || (secretNow(e) && !!secrets.add(e));
  // What kind of secret, as its input_type: a one-time code or a password.
  const secretKind = e => /one-time-code$/i.test(e.getAttribute('autocomplete')||'') ?
    'one-time-code' : 'password';
  const offerable = e => !['file','hidden'].includes(e.type);
  const safe = e => offerable(e) && !secret(e);
  // A native select made transparent over a styled box is still what takes
  // the click, so its opacity does not hide it. An element with display:
  // contents has no box of its own (checkVisibility is false): it shows what
  // its parent shows.
  const shows = e => e.checkVisibility(e.tagName==='SELECT' ? {checkVisibilityCSS:true} :
    {checkOpacity:true,checkVisibilityCSS:true});
  const visible = e => !e.closest('[aria-hidden="true"],[inert]') && (shows(e) ||
    (tree.contents(e) && getComputedStyle(e).visibility!=='hidden' && !!tree.up(e) && visible(tree.up(e))));
  const roles=['button','link','checkbox','radio','switch','tab','menuitem','menuitemradio',
    'menuitemcheckbox','option','treeitem','gridcell','combobox','textbox','searchbox','spinbutton'];
  // An anchor with no href acts through its click handler alone (a jQuery UI
  // datepicker's Next), and a label is offered for a styled checkbox or radio
  // whose own input is hidden; both follow fastbrowse's snapshot.js (MIT).
  const controls='a[href],a[onclick],button,input,textarea,select,summary,'+
    '[contenteditable="true"],'+roles.map(role=>'[role="'+role+'"]').join(',');
  const selector=controls+',label';
  const {name,field,squash,cut,regionName}=parts.names({visible,controls});
  // Only a primitive goes into a guard: a custom element's value getter may
  // return an object, even a cyclic one, that cannot be returned by value.
  const plain=v=>{
    if (v==null || ['string','number','boolean'].includes(typeof v)) return v ?? null;
    try { return String(v); } catch { return null; }
  };
  // A value as text, well formed: a lone surrogate (half an emoji) would
  // reach DevTools as an unpaired escape.
  const asText=v=>{ try { return String(v).toWellFormed(); } catch { return ''; } };
  // Anything else a person would click (Jev's own): an element with a click
  // listener (read through DevTools' getEventListeners, which observe()
  // exposes), an onclick handler, a tabindex, or the outermost element of a
  // region drawn with a pointer cursor. Not one inside a control, one larger
  // than a third of the viewport (a delegation root, a page wrapper), or one
  // that only wraps controls (see `wraps` below).
  const generic=new WeakSet();
  const CLICKS=['click','mousedown','mouseup','pointerdown','pointerup'];
  const inert=['SCRIPT','STYLE','NOSCRIPT','TEMPLATE','LINK','META','BR','HR','IFRAME','OPTION',
    'OPTGROUP','LABEL'];
  const listens=e=>{
    if (e.onclick || e.hasAttribute('onclick')) return true;
    if (typeof getEventListeners!=='function') return false;
    try { const l=getEventListeners(e); return CLICKS.some(t=>l?.[t]?.length); } catch { return false; }
  };
  // A tabindex alone makes a thing focusable, not clickable: not one with a
  // role of its own (a slider, a tab panel, a dialog) or a block of code.
  const tabbable=e=>e.hasAttribute('tabindex') && e.tabIndex>=0 && !e.hasAttribute('role') &&
    e.tagName!=='PRE';
  // The outermost element of a pointer region; a shadow root's element is
  // compared with its host.
  const pointer=e=>{
    if (getComputedStyle(e).cursor!=='pointer') return false;
    const parent=e.parentElement || e.parentNode?.host;
    return !parent || getComputedStyle(parent).cursor!=='pointer';
  };
  // Inside a control, through shadow roots: an icon element in a button's
  // light DOM draws its picture in a shadow root of its own.
  const inControl=e=>{
    if (e.parentElement?.closest(controls)) return true;
    for (let root=e.getRootNode(); root.host; root=root.host.getRootNode())
      if (root.host.closest(controls)) return true;
    return false;
  };
  const clickable=e=>{
    if (inert.includes(e.tagName)) return false;
    if (!(listens(e) || tabbable(e) || pointer(e))) return false;
    if (inControl(e) || !visible(e)) return false;
    const r=rectOf(e);
    if (!r || r.width*r.height>innerWidth*innerHeight/3) return false;
    // A clickable nested in another that reads the same (a menu item's
    // wrapper) is the same control, offered once as the outer one.
    for (let p=e.parentElement, depth=0; p && depth<3; p=p.parentElement, depth++)
      if (generic.has(p) && p.textContent.trim()===e.textContent.trim()) return false;
    return true;
  };
  // An item of a floating list (an autocomplete's suggestions, a dropdown)
  // with no role of its own is offered as an option.
  const floating=e=>{
    const list=e.closest('ul,ol,[role="listbox"],[role="menu"]');
    for (let x=list; x && x!==document.body; x=x.parentElement)
      if (['absolute','fixed'].includes(getComputedStyle(x).position)) return true;
    return false;
  };
  // Native date and time inputs take their type's ISO value; fill.js sets it.
  const dates=['date','datetime-local','month','week','time'];
  const choice = e => e?.tagName==='INPUT' && ['checkbox','radio'].includes(e.type);
  // A checkbox or radio drawn by its label: the input is transparent, not
  // rendered, or too small to click.
  const styled = e => {
    if (!choice(e)) return false;
    const r=e.getBoundingClientRect();
    return !visible(e) || r.width<2 || r.height<2;
  };
  const shownLabel = e => [...(e.labels||[])].some(l=>visible(l));
  // The label of a hidden checkbox or radio a person could still toggle.
  const labelled = e => e.tagName==='LABEL' && styled(e.control) && !e.control.matches(':disabled') &&
    !e.control.closest('[aria-disabled="true"],[inert]');
  const source = e => e.tagName==='LABEL' && e.control ? e.control : e;
  const role = e => {
    const explicit=e.getAttribute('role');
    if (roles.includes(explicit)) return explicit;
    if (e.tagName==='LABEL') return labelled(e) ? e.control.type : null;
    if (e.tagName==='BUTTON' || e.tagName==='SUMMARY') return 'button';
    if (e.tagName==='A') return e.hasAttribute('href') ? 'link' : 'button';
    if (e.tagName==='SELECT') return 'combobox';
    if (e.tagName==='TEXTAREA' || e.isContentEditable) return 'textbox';
    if (e.tagName==='INPUT') {
      if (['checkbox','radio'].includes(e.type)) return e.type;
      if (['button','submit','reset','image'].includes(e.type)) return 'button';
      if (e.type==='search') return 'searchbox';
      if (e.type==='number') return 'spinbutton';
      if (['text','email','url','tel','password',...dates].includes(e.type)) return 'textbox';
    }
    if (!generic.has(e)) return null;
    return e.tagName==='LI' && floating(e) ? 'option' : 'button';
  };
  // context.js runs on this snapshot's result and names controls the same way,
  // and act.js hit-tests against the same generic clickables.
  cache.visible=visible; cache.name=name; cache.generic=generic; cache.tree=tree;
  // The document, then the open shadow roots and same-origin frames below.
  const roots=[document];
  const every=selector=>roots.flatMap(root=>[...root.querySelectorAll(selector)]);
  // Only whether a secret is filled, never its value.
  const held=e=>secret(e) ? e.value!=='' : e.value;
  cache.pageKey=()=>[performance.timeOrigin,location.href,scrollX,scrollY,innerWidth,innerHeight,
    roots.flatMap(root=>[...root.querySelectorAll('input,textarea,select')]).filter(offerable)
      .map(e=>[identity(e),held(e),e.checked,e.selectedIndex,e.disabled,e.readOnly])];
  cache.guard=e=>{
    if (!e?.isConnected || !visible(e)) return null;
    // A generic clickable is its own scope: its parent may be a grid of a
    // thousand cards.
    const scope=e.closest('form,dialog,[role="dialog"],article,li,tr,[role="row"]') ||
      (generic.has(e) ? e : e.parentElement);
    // A label's value and checked state are its checkbox's.
    return [identity(e),role(e),asText(name(e)),plain(held(source(e))),plain(source(e).checked),
      plain(e.selectedIndex),plain(e.readOnly),e.matches(':disabled'),
      // Only "true" disables (as act.js reads it), so a page hydrating a
      // missing aria-disabled to "false" does not make a decision stale.
      e.getAttribute('aria-disabled')==='true',
      e.getAttribute('aria-expanded'),e.getAttribute('aria-checked'),e.getAttribute('aria-selected'),
      e.getAttribute('href'),e.form ? identity(e.form) : null,cut(scope?.innerText||'',6000).toWellFormed()];
  };
  // Pagers and load-more controls end the list they continue, past any cap
  // that counts from the viewport, so they are always kept. The narrow rules
  // (a link to an address marked rel=next or labelled Next, or a control
  // labelled "Show 20 more") follow fastbrowse's page.py (MIT).
  const nextPage=/^(?:(?:next(?: page)?|more results|older(?: posts)?)\s*[›»→>]*|[›»→>]{1,2})$/i;
  const loadMore=/^(?:show|view|load|see)(?: \d+)? more(?: [\p{L}\p{N}_-]+){0,3}$/iu;
  const {fixed,reachable,clipped}=parts.reach({tree,rectOf});
  const pagers=new Set(), distance=new Map(), deferred=new Map();
  const pages=(e,rname,label)=>loadMore.test(label) || (e.tagName==='A' && rname==='link' &&
    !!e.getAttribute('href') && (e.relList.contains('next') || nextPage.test(label)));
  // A generic clickable's text can be a whole card's; it is cut to 100.
  const genericName=e=>{
    const named=name(e);
    if (!generic.has(e)) return named;
    const text=squash(named);
    return text.length>100 ? cut(text,99)+'…' : text;
  };
  const actions=[];
  // Boxes that scroll inside themselves (Jev's own): a terms box, a
  // textarea, a list in a panel. Up to ten on screen are offered to scroll
  // down or up, so what they hold can be read and reached.
  const regions=[];
  const scrolls=e=>{
    if (regions.length>=10 || e.clientHeight<24 || e.scrollHeight<=e.clientHeight+8) return;
    if (!['auto','scroll'].includes(getComputedStyle(e).overflowY) || !visible(e)) return;
    const r=rectOf(e), x=r && r.x+r.width/2;
    if (r && r.bottom>0 && r.top<innerHeight && x>=0 && x<innerWidth) regions.push(e);
  };
  // Controls and generic clickables in document order, a shadow root's or a
  // frame's after its host. What sits in a control's shadow is its inside (a
  // custom button's <button>), and so is the shadow of anything inside a
  // control (an icon element in a button draws its picture there): neither
  // is offered again.
  const standard=new Set(), found=[];
  const walk=(root,inside)=>{
    if (root!==document) { roots.push(root); cache.watch?.(root); }
    for (const e of root.querySelectorAll(selector)) standard.add(e);
    const all=root.nodeType===9 ? root.body?.getElementsByTagName('*') ?? [] : root.querySelectorAll('*');
    for (const e of all) {
      scrolls(e);
      if (!inside && standard.has(e)) found.push(e);
      else if (!inside && clickable(e)) { generic.add(e); found.push(e); }
      if (e.shadowRoot)
        walk(e.shadowRoot,inside || standard.has(e) || generic.has(e) || inControl(e));
      else if (e.tagName==='IFRAME' || e.tagName==='FRAME') {
        const doc=tree.frameDoc(e);
        if (doc?.body && visible(e) && tree.place(doc,placed)) walk(doc,false);
      }
    }
  };
  walk(document,false);
  // A clickable that only wraps controls, generic ones included (a list
  // listening for its rows' clicks, an item around its link), is not
  // offered: every word of it is theirs, or it holds more than four.
  const holds=new Map();
  for (const e of found) for (let p=e.parentElement; p && p!==document.body; p=p.parentElement)
    if (generic.has(p)) holds.set(p,(holds.get(p)||0)+1);
  const theirs=(t,e)=>{
    for (let x=t.parentElement; x && x!==e; x=x.parentElement)
      if (generic.has(x) || x.matches(controls)) return true;
    return false;
  };
  const wraps=e=>{
    const held=holds.get(e)||0;
    if (!held) return false;
    if (held>4) return true;
    const walk=e.ownerDocument.createTreeWalker(e,NodeFilter.SHOW_TEXT);
    for (let t=walk.nextNode(); t; t=walk.nextNode())
      if (t.textContent.trim() && !theirs(t,e)) return false;
    return true;
  };
  for (let i=found.length-1; i>=0; i--)
    if (generic.has(found[i]) && wraps(found[i])) { generic.delete(found[i]); found.splice(i,1); }
  // A cheap pass keeps what is shown, enabled and in reach sideways; the
  // rest of the work is paid only for controls the caps can keep.
  const entries=[];
  for (const e of found) {
    if (!offerable(e) || !visible(e) || e.matches(':disabled') || e.closest('[aria-disabled="true"]')) continue;
    // A styled checkbox is offered once, through its label.
    if (styled(e) && shownLabel(e)) continue;
    const r=rectOf(e), rname=role(e);
    if (!r) continue;
    const x=r.x+r.width/2, y=r.y+r.height/2, clip=tree.place(e.ownerDocument,placed)?.clip;
    // Controls above or below the viewport are kept and flagged: act.js
    // scrolls to them. Anything off to the side is not, nor anything a
    // frame's box cuts off (the frame's own scrolling is not reached).
    if (!rname || r.width<=0 || r.height<=0 || x<0 || x>=innerWidth) continue;
    if (clip && (x<clip.left || x>=clip.right || y<clip.top || y>=clip.bottom)) continue;
    if (rname==='gridcell' && e.querySelector('button,[role="button"]')) continue;
    entries.push({e,r,y,rname,offscreen:y<0 || y>=innerHeight});
  }
  // Controls no scroll can reach, or that an ancestor cuts off, are counted
  // as omitted (see `omitted_actions` below).
  let unreached=0;
  // Builds an entry's actions, returning how many controls it added: a
  // select counts once, however many options it has.
  const build=({e,r,y,rname,offscreen})=>{
    if ((offscreen && (!reachable(e,r) || fixed(e))) || clipped(e,r)) { unreached++; return 0; }
    // An offscreen generic clickable is named once it is kept: a page can
    // hold thousands of clickable cards below the fold.
    const later=generic.has(e) && offscreen;
    const base={node:identity(e),role:rname,label:later ? '' : asText(genericName(e)||rname),
      rect:{x:r.x,y:r.y,w:r.width,h:r.height}};
    for (const key of ['checked','selected','expanded']) {
      const value=e.getAttribute('aria-'+key);
      if (value!==null) base[key]=value;
    }
    if (choice(source(e))) base.checked=String(source(e).checked);
    // What kind of field it is (Jev's own): a textarea and a one-line input
    // both read as "textbox" otherwise.
    if (e.tagName==='TEXTAREA') base.input_type='textarea';
    else if (e.tagName==='INPUT' && ['textbox','searchbox','spinbutton','combobox'].includes(rname))
      base.input_type=e.type;
    // A secret says what kind it is and whether it is filled, never what it holds.
    const hidden=['INPUT','TEXTAREA'].includes(e.tagName) && secret(e);
    if (hidden) { base.input_type=secretKind(e); base.filled=e.value!==''; }
    if (offscreen) base.offscreen=true;
    const first=actions.length;
    if (e.tagName==='SELECT') {
      // The chosen option too (Jev's own): asked for the value a select
      // already shows, the model answered BLOCKED when it was not listed.
      for (const o of e.options) if (!o.disabled && !o.closest('optgroup[disabled]'))
        actions.push({...base,kind:'select',value:o.value,
          current_value:asText([...e.selectedOptions].map(o=>o.label).join(', ')),
          label:asText(base.label+' → '+o.label)});
    } else {
      const editable=!e.readOnly && e.getAttribute('aria-readonly')!=='true' &&
        (['textbox','searchbox','spinbutton'].includes(rname) ||
          (rname==='combobox' && ['INPUT','TEXTAREA'].includes(e.tagName)));
      // A value that is not text (a custom element's object) is not shown.
      const own='value' in e ? e.value : undefined;
      const value=generic.has(e) || hidden ? '' : ['string','number'].includes(typeof own) ? asText(own) :
        e.isContentEditable || ['combobox','textbox','searchbox'].includes(rname) ?
          asText(e.innerText.trim()) : '';
      actions.push({...base,kind:editable?'fill':'click',value});
      if (editable) actions.push({...base,kind:'click',value,label:'Open '+base.label});
    }
    if (later) deferred.set(actions[first],e);
    const pager=pages(e,rname,later ? squash(e.textContent) : squash(base.label));
    for (const a of actions.slice(first)) {
      distance.set(a,offscreen ? 1+Math.abs(y-innerHeight/2) : 0);
      if (pager && a.kind==='click') pagers.add(a);
    }
    return e.tagName==='SELECT' ? Math.min(1,actions.length-first) : actions.length-first;
  };
  for (const entry of entries) if (!entry.offscreen) build(entry);
  // Offscreen controls nearest first: past the first 100 controls only a
  // pager can survive the caps, so only pagers are built.
  const below=entries.filter(entry=>entry.offscreen)
    .map((entry,order)=>({...entry,order,far:Math.abs(entry.y-innerHeight/2)}))
    .sort((a,b)=>a.far-b.far || a.order-b.order);
  let offered=0, skipped=0;
  for (const entry of below) {
    if (offered<100) offered+=build(entry);
    else if (pages(entry.e,entry.rname,squash(generic.has(entry.e) ? entry.e.textContent : name(entry.e))))
      build(entry);
    else skipped++;
  }
  // Form ownership is observable structure, not an inferred action meaning.
  // Keep values out: secret fields contribute only their labels.
  const formEvidence=new Map();
  for (const a of actions) {
    const node=cache.nodes.get(a.node);
    if (!node) continue;
    const e=source(node), form='form' in e ? e.form : e.closest('form');
    if (form && !formEvidence.has(form)) {
      const controls=[...form.elements];
      formEvidence.set(form,{id:identity(form),fields:controls.filter(x=>field(x) && visible(x)).map(x=>asText(name(x))).slice(0,12),
        submit_buttons:controls.filter(x=>['submit','image'].includes(x.type) && offerable(x)).map(x=>asText(name(x))).slice(0,8)});
    }
    a.form=form ? formEvidence.get(form) : null;
  }
  const disabled_controls=every(':disabled,[aria-disabled="true"]').filter(visible)
    .map(e=>({label:asText(name(e)),role:role(e)})).filter(e=>e.label).slice(0,20);
  const readText=parts.text({tree,visible,safe,placed,roots,regions,rectOf,cut});
  const {text:pageText,held:typed_values}=readText(), height=document.documentElement.scrollHeight;
  const page_key=cache.pageKey(), guards={};
  // Compare meaning and identity. Geometry is always resolved and hit-tested just before input.
  // Only what is on screen counts: a list growing below the fold must not make a decision stale.
  const semantics=actions.filter(a=>!a.offscreen).map(({rect,...action})=>action);
  const marker=[performance.timeOrigin,location.href,scrollX,scrollY,innerWidth,innerHeight,
    document.title,pageText,disabled_controls,semantics,page_key[6]];
  // Nearest first, in document order on screen (the sort is stable), then cap
  // offscreen controls on their own and everything together, keeping pagers.
  // The caps count controls: a select's options are one, so a long list of
  // countries cannot crowd out the form's Submit button.
  actions.sort((a,b)=>distance.get(a)-distance.get(b));
  const unit=a=>a.kind==='select' ? 'select '+a.node : a;
  const capped=(list,limit)=>{
    if (new Set(list.map(unit)).size<=limit) return list;
    const keep=new Set(list.filter(a=>pagers.has(a)).map(unit).slice(0,limit));
    for (const a of list) { if (keep.size>=limit) break; keep.add(unit(a)); }
    return list.filter(a=>keep.has(unit(a)));
  };
  const kept=capped([...actions.filter(a=>!a.offscreen),
    ...capped(actions.filter(a=>a.offscreen),100)],250);
  // The controls left out, one each: a select with 40 options or a field with
  // its "Open" click is one control, not 40 or 2 actions. The key keeps its
  // upstream name.
  const listed=new Set(kept.map(a=>a.node));
  const omitted_actions=new Set(actions.filter(a=>!listed.has(a.node)).map(a=>a.node)).size+
    skipped+unreached;
  actions.splice(0,actions.length,...kept);
  for (const a of actions) if (deferred.has(a)) a.label=asText(genericName(deferred.get(a))||a.role);
  // Where each box is scrolled to: top, bottom, or how far down in tenths,
  // so every scroll is progress the step fingerprint sees.
  for (const e of regions) {
    const top=e.scrollTop<=1, end=e.scrollTop+e.clientHeight>=e.scrollHeight-2;
    const rname=field(e) ? role(e)||'textbox' : 'region';
    const part=Math.round(e.scrollTop/(e.scrollHeight-e.clientHeight)*10)*10;
    const base={node:identity(e),role:rname,label:asText(regionName(e)||rname),
      scrolled:top ? 'top' : end ? 'bottom' : Math.min(90,Math.max(10,part))+'%'};
    if (!end) actions.push({...base,kind:'scroll',direction:'down'});
    if (!top) actions.push({...base,kind:'scroll',direction:'up'});
  }
  // Keys (Jev's own). Enter goes only to the field Jev last typed into,
  // while it has focus and holds text: it can submit a form or send a
  // message, so it is never offered for a field Jev did not fill. Escape is
  // offered only while something is open: an expanded popup or combobox, a
  // dialog, or a floating list or menu (not an expanded accordion or tab).
  const active=tree.active();
  const entered=active && active===cache.typed &&
    actions.find(a=>a.kind==='fill' && cache.nodes.get(a.node)===active && (a.filled || String(a.value).trim()));
  if (entered) actions.push({...entered,kind:'enter',context:undefined,offscreen:undefined});
  const open=every('[aria-expanded="true"][aria-haspopup]:not([aria-haspopup="false"]),'+
    '[role="combobox"][aria-expanded="true"],dialog[open],[role="dialog"],[role="alertdialog"],'+
    '[aria-modal="true"]').some(x=>visible(x)) ||
    every('[role="listbox"],[role="menu"]').some(x=>visible(x) && floating(x)) ||
    actions.some(a=>a.role==='option' && generic.has(cache.nodes.get(a.node)));
  for (const a of actions) if (!(a.node in guards)) guards[a.node]=cache.guard(cache.nodes.get(a.node));
  actions.forEach((a,i)=>a.id='e'+(i+1));
  if (scrollY+innerHeight<height-2) actions.push({id:'scroll_down',kind:'scroll',label:'Scroll down',delta:560});
  if (scrollY>0) actions.push({id:'scroll_up',kind:'scroll',label:'Scroll up',delta:-560});
  if (open) actions.push({id:'press_escape',kind:'key',key:'Escape',
    label:'Press Escape to close the open popup, menu, suggestion list or dialog'});
  actions.push({id:'wait',kind:'wait',label:'Wait for the page to update'});
  return {url:location.href,title:document.title.toWellFormed(),w:innerWidth,h:innerHeight,text:pageText,
    typed_values,disabled_controls,scroll:{y:scrollY,height},actions,marker,page_key,guards,omitted_actions};
})
