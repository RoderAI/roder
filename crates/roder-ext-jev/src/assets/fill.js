// The steps of a fill around the typed text (jev-owned). Typing goes where a
// person's would: to the field the click focused, with its content selected
// by script rather than by a select-all accelerator a page can intercept,
// and the text is then checked to have stayed. The hand-off, focus and
// landing rules, and setting native date inputs through the value setter,
// follow fastbrowse's browser/page.py (MIT).
({
  // After the click, the field that should take the text. A field that opens
  // an editor over itself (a search overlay, an airport picker) moves focus
  // to that editor a frame or a timer later, so this waits for the page to
  // be quiet for 100 ms, 600 ms at most, then follows focus to a visible,
  // enabled, editable field that covers the clicked point. A clicked field
  // that hid itself is being replaced, so the wait goes on for its editor.
  // Returns [node, date]: that field's node id (the clicked field's own when
  // focus went nowhere else) and whether it is a native date or time input;
  // or null when the clicked field is gone or hidden and nothing took its
  // place.
  handoff: ({node, x, y}) => new Promise(resolve => {
    const c=window.__jevFast, e=c?.nodes.get(node);
    if (!e) return resolve(null);
    const DATES=['date','datetime-local','month','week','time'];
    const NOT_TEXT=['button','checkbox','color','file','hidden','image','password','radio','range',
      'reset','submit'];
    const identity=a=>{
      if (!c.ids.has(a)) c.ids.set(a,c.next++);
      const id=c.ids.get(a); c.nodes.set(id,a); return id;
    };
    const answer=a=>[identity(a), a.tagName==='INPUT' && DATES.includes(a.type)];
    const shown=a=>a.checkVisibility({checkOpacity:true,checkVisibilityCSS:true});
    const own=()=>e.isConnected && shown(e) ? answer(e) : null;
    const tree=c.tree;
    const decide=()=>{
      const a=tree.active();
      if (!a || a===a.ownerDocument.body || a===e || tree.within(a,e)) return own();
      const text=a.isContentEditable || a.tagName==='TEXTAREA' ||
        (a.tagName==='INPUT' && !NOT_TEXT.includes(a.type));
      if (!text || !a.isConnected || a.disabled || a.readOnly ||
          a.closest('[aria-disabled="true"],[aria-readonly="true"],[inert]') ||
          !shown(a)) return own();
      const r=tree.rect(a);
      if (!r || x<r.left || x>r.right || y<r.top || y>r.bottom) return own();
      return answer(a);
    };
    const deadline=performance.now()+600;
    const poll=()=>{
      const choice=decide();
      if ((choice!==null && performance.now()-(c.lastMutation ?? 0)>=100) || performance.now()>=deadline)
        resolve(choice);
      else setTimeout(poll,20);
    };
    let started=false;
    const start=()=>{ if (!started) { started=true; poll(); } };
    // Look after the click's own frame; the timer covers a page that draws none.
    setTimeout(start,600);
    if (document.hidden) start(); else requestAnimationFrame(()=>setTimeout(start,0));
  }),
  // Give the field keyboard focus and select what it holds, so the typed
  // text replaces it. Returns whether the field holds focus.
  focus: node => {
    const e=window.__jevFast?.nodes.get(node), tree=window.__jevFast?.tree;
    if (!e?.isConnected || !tree) return false;
    const active=tree.active;
    const holds=()=>active()===e || (e.isContentEditable && e.contains(active()));
    if (!holds()) e.focus({preventScroll:true});
    if (!holds()) return false;
    // The field Jev is typing into: the only one snapshot.js offers Enter in.
    window.__jevFast.typed=e;
    // The suggestions it shows before the text is typed, for settle.js.
    window.__jevFast.suggested=window.__jevFast.suggestions?.(e);
    if (typeof e.select==='function') e.select();
    else {
      const range=e.ownerDocument.createRange(), selection=e.ownerDocument.defaultView.getSelection();
      range.selectNodeContents(e);
      selection.removeAllRanges();
      selection.addRange(range);
    }
    return true;
  },
  // Whether the field holds exactly the text, or a focused field that took
  // its place at the clicked point does (a framework can swap in a hydrated
  // copy while the text is typed). Returns [landed, what the field shows],
  // or only [landed] for a secret (`secret`, or a field snapshot.js saw as
  // one): what it holds never leaves the page, and it stays a secret.
  landed: ({node, text, x, y, secret}) => {
    const c=window.__jevFast, e=c?.nodes.get(node);
    const hidden=secret || (e && c.secrets?.has(e));
    if (hidden && e) (c.secrets ||= new WeakSet()).add(e);
    const shown=n=>String(n.value ?? n.innerText ?? '');
    // What the field keeps of the text: typed into a single-line input, a
    // line break becomes a space, and an email or URL input also trims the
    // ends, as Chrome does; a textarea keeps "\n" for "\r\n".
    const kept=n=>{
      if (n.tagName==='TEXTAREA') return text.replace(/\r\n?/g,'\n');
      if (n.tagName!=='INPUT') return text;
      const single=text.replace(/\r\n|[\r\n]/g,' ');
      return ['email','url'].includes(n.type) ? single.replace(/^[\t\n\f\r ]+|[\t\n\f\r ]+$/g,'') : single;
    };
    // A field that formats what it holds (a phone mask adding brackets and
    // dashes, a case change) kept the text when the same letters and digits
    // are there in the same order.
    const plain=v=>v.replace(/[^\p{L}\p{N}]/gu,'').toLowerCase();
    const holds=n=>{
      const value=shown(n), want=kept(n);
      return value===want || (plain(want)!=='' && plain(value)===plain(want));
    };
    if (e?.isConnected && holds(e)) return [true];
    const a=c?.tree?.active();
    if (a && a!==e && a!==a.ownerDocument.body) {
      const r=c.tree.rect(a);
      if (r && x>=r.left && x<=r.right && y>=r.top && y<=r.bottom && holds(a)) {
        if (hidden) (c.secrets ||= new WeakSet()).add(a);
        return [true];
      }
    }
    if (hidden) return [false];
    const value=e?.isConnected ? shown(e) : null;
    return [false, value===null ? null : Array.from(value).slice(0,100).join('').toWellFormed()];
  },
  // A native date or time input keeps its type's ISO shape. Typed keys land
  // in whichever locale-formatted segment has focus, so the value goes
  // through the setter frameworks patch, then the input and change events
  // the page's handlers expect. Returns [the value the field kept, whether
  // it is the same moment as the value given], or null when it is gone.
  // The input writes a moment its own way ("14:30:00" is kept as "14:30"),
  // so the two are compared as a detached input of the same type reads them.
  date: ({node, value}) => {
    const e=window.__jevFast?.nodes.get(node);
    if (!e?.isConnected) return null;
    // The setter of the input's own window, which a framework in a frame patches.
    const own=e.ownerDocument.defaultView.HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(own,'value').set.call(e,value);
    e.dispatchEvent(new Event('input',{bubbles:true}));
    e.dispatchEvent(new Event('change',{bubbles:true}));
    const probe=e.ownerDocument.createElement('input');
    probe.type=e.type;
    probe.value=value;
    const same=e.value===value || (probe.value!=='' && e.value!=='' &&
      probe.valueAsNumber===e.valueAsNumber);
    return [e.value, same];
  },
})
