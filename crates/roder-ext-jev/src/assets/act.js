// Started from upstream Jev's browser.py (MIT, browser-use/jev-ultrafast); now
// Jev-owned. Resolves an observed node to a hit-tested click point, with no
// side effect but scrolling. Returns null for a target that went away, is
// hidden or disabled, or cannot be brought into the viewport (a stale page),
// {covered:true, by, controls, opens} when it is there but another element would
// take the click (what covers it and what that offers to press), and
// otherwise {x, y, scrolled, announced, opens}: whether it scrolled, whether
// the target says a click opens a popup (a menu, a listbox, a dialog) that
// the settle should wait for, and whether it says a click opens a new
// window, whose tab page/tabs.rs then waits for. Node ids are code-owned and always refer to an
// observed element, never to a model-generated selector. Scrolling only when
// needed, the multi-point, nested-control hit test and the popup
// announcement follow fastbrowse's browser/page.py (MIT). Choosing a select
// option is select.js, run after this. A target inside an open shadow root
// or a same-origin frame is measured and hit-tested through tree.js (kept by
// snapshot.js): points are in the top window, and the hit descends through
// shadow roots and frames.
(action => {
  const c=window.__jevFast, e=c?.nodes.get(action.node), tree=c?.tree;
  // Visible as snapshot.js reads it: a transparent native select over a
  // styled box, and a display:contents link, are visible.
  if (!tree || !e?.isConnected || e.matches(':disabled') || e.closest('[aria-disabled="true"],[inert]') ||
      !c.visible(e)) return null;
  if (action.kind==='fill' && (e.readOnly || e.getAttribute('aria-readonly')==='true')) return null;
  const r=tree.rect(e);
  if (!r?.width || !r.height) return null;
  // The part of the window the target's frame shows; all of it at the top.
  const view=()=>{
    const clip=tree.place(e.ownerDocument)?.clip;
    const all={left:0,top:0,right:innerWidth,bottom:innerHeight};
    return !clip ? all : {left:Math.max(0,clip.left),top:Math.max(0,clip.top),
      right:Math.min(innerWidth,clip.right),bottom:Math.min(innerHeight,clip.bottom)};
  };
  // Scroll only a target not fully in view: a scroll closes open menus, so
  // centring an option that was already visible would dismiss it. In a
  // frame this scrolls the frame and the window both.
  let scrolled=false;
  const v=view();
  if (r.top<v.top || r.left<v.left || r.bottom>v.bottom || r.right>v.right) {
    e.scrollIntoView({block:'center',inline:'nearest',behavior:'instant'});
    scrolled=true;
  }
  // A wrapped link has one rect per line; bound the search to four.
  const clipped=()=>{
    const v=view();
    return tree.rects(e).map(r=>({left:Math.max(v.left,r.left),top:Math.max(v.top,r.top),
      right:Math.min(v.right,r.right),bottom:Math.min(v.bottom,r.bottom)}))
      .filter(r=>r.right>r.left && r.bottom>r.top).slice(0,4);
  };
  if (!clipped().length) return null;
  // Every control snapshot.js can offer, including the label of a styled
  // checkbox or radio. One nested inside the target (a row's Delete button)
  // takes the click itself, so a point belongs to the target only when no
  // such control sits between the hit and the target. A label's own hidden
  // input is the label's click, not another control.
  // The last snapshot's generic clickables count too: a star icon in a
  // clickable mail row takes its own click.
  const roles=['button','link','checkbox','radio','switch','tab','menuitem','menuitemradio',
    'menuitemcheckbox','option','treeitem','gridcell','combobox','textbox','searchbox','spinbutton'];
  const controls='a[href],a[onclick],button,input,textarea,select,summary,[contenteditable="true"],'+
    roles.map(role=>'[role="'+role+'"]').join(',');
  const generic=c.generic;
  // What sits in the target's own shadow root is the target's inside (a
  // custom button's <button>), not another control.
  const own=x=>{
    for (let root=x.getRootNode(); root.host; root=root.host.getRootNode()) if (root.host===e) return true;
    return false;
  };
  const control=x=>x!==e.control && !own(x) && (x.matches(controls) || !!generic?.has(x) ||
    (x.tagName==='LABEL' && ['checkbox','radio'].includes(x.control?.type)));
  const inner=hit=>{
    let found=null;
    for (let x=hit; x && x!==e; x=tree.up(x)) if (control(x)) found=x;
    return found;
  };
  const owns=hit=>!!hit && tree.within(hit,e) && !inner(hit);
  // A nested control that fills the target (a combobox's own input, a grid
  // cell's link) is where a person's click on the target lands too.
  const area=b=>b ? b.width*b.height : 0;
  const fills=hit=>{
    const x=hit && tree.within(hit,e) ? inner(hit) : null;
    return !!x && area(tree.rect(x))>=0.75*area(tree.rect(e));
  };
  // The centre first, then the four quarter points, of each rect. `cover`
  // keeps what took the first point that was not the target's.
  let cover=null;
  const find=test=>{
    for (const c of clipped()) {
      for (const [fx,fy] of [[.5,.5],[.25,.25],[.75,.25],[.25,.75],[.75,.75]]) {
        const x=c.left+(c.right-c.left)*fx, y=c.top+(c.bottom-c.top)*fy;
        // tree.hit starts from document.elementFromPoint.
        const hit=tree.hit(x,y);
        if (test(hit)) return {x,y};
        cover ||= hit;
      }
    }
    return null;
  };
  let point=find(owns);
  if (!point) {
    // A scrollable ancestor may clip the target while the window shows it.
    // 'nearest' moves nothing for a target already in view, so an open menu
    // stays open.
    e.scrollIntoView({block:'nearest',inline:'nearest',behavior:'instant'});
    scrolled=true;
    cover=null;
    point=find(owns) || find(fills);
  }
  // The name the page gives what covers the target: its label, else its
  // text, else its tag, from the element hit or the nearest of its first
  // few ancestors that has one. This is page text, as untrusted as the rest:
  // the caller scrubs it, puts it on one line and cuts it. An ancestor of
  // the target is no cover's name (its text holds the target's own), and a
  // field's value is never read, bar a button's own label.
  const said=x=>{
    for (let n=x, i=0; n && i<4 && n!==n.ownerDocument.body && n!==n.ownerDocument.documentElement;
         n=tree.up(n), i++) {
      const holds=tree.within(e,n);
      const field=n.matches('input,textarea,select');
      const button=n.matches('input[type="button"],input[type="submit"],input[type="reset"]');
      const name=(n.getAttribute('aria-label')||n.getAttribute('title')||n.getAttribute('alt')||
        (field ? n.getAttribute('placeholder') : '')||(button ? n.value : '')||
        (holds||field ? '' : n.innerText)||'').replace(/\s+/g,' ').trim();
      if (name) return name.slice(0,400);
      if (holds) break;
    }
    return null;
  };
  // What the cover offers to press, so a caller or the fallback can close
  // it: the visible buttons and links of the layer that holds it (the
  // largest branch holding what was hit but not the target), or, when that
  // holds none (a bare backdrop beside its dialog), of an open dialog or
  // popover elsewhere on the page, which then names a cover that names
  // nothing itself; eight at most, and as untrusted as the name.
  const offered=hit=>{
    let layer=hit;
    for (let x=tree.up(hit); x && !tree.within(e,x) && x!==x.ownerDocument.body &&
         x!==x.ownerDocument.documentElement; x=tree.up(x)) layer=x;
    const PRESS='button,[role="button"],a[href],input[type="button"],input[type="submit"]';
    const listed=root=>[...(root.matches?.(PRESS) ? [root] : []), ...root.querySelectorAll(PRESS)]
      .filter(x=>!tree.within(e,x) && !tree.within(x,e) && c.visible(x));
    let found=layer.nodeType===1 ? listed(layer) : [], from=layer;
    if (!found.length) {
      for (const d of layer.ownerDocument.querySelectorAll('dialog[open],[role="dialog"],'+
           '[role="alertdialog"],[aria-modal="true"],:popover-open')) {
        if (tree.within(e,d) || !c.visible(d)) continue;
        found=listed(d);
        if (found.length) { from=d; break; }
      }
    }
    const named=x=>(x.getAttribute('aria-label')||x.getAttribute('title')||
      (x.matches('input') ? x.value : x.innerText)||'').replace(/\s+/g,' ').trim().slice(0,80);
    return {from, controls:[...new Set(found.map(named).filter(Boolean))].slice(0,8)};
  };
  // A click that opens a new window: a link, or a form's submit, that names
  // a new browsing context (as fastbrowse's page/act.rs reads it), or a
  // control that says it opens a new tab or window, as accessible ones do,
  // or whose own handler calls window.open. Its tab may be announced well
  // after the settle, so page/tabs.rs waits for it.
  const fresh=name=>(name ?? '').trim().toLowerCase()==='_blank';
  const link=e.closest('a[href],area[href]');
  const says=[e.getAttribute('aria-label'), e.getAttribute('title'), e.innerText, link?.getAttribute('title')]
    .some(t=>/\bnew (tab|window)\b/i.test(t||''));
  const handler=[e, e.parentElement].some(x=>/\bwindow\.open\s*\(/.test(x?.getAttribute('onclick')||''));
  const opens=(!!link && fresh(link.hasAttribute('target') ? link.getAttribute('target')
      : e.ownerDocument.querySelector('base[target]')?.getAttribute('target'))) ||
    (!!e.form && ['submit','image'].includes(e.type) &&
      fresh(e.getAttribute('formtarget') ?? e.form.getAttribute('target'))) || says || handler;
  if (!point) {
    if (!cover) return {covered:true, by:null, controls:[], opens};
    const {from, controls}=offered(cover);
    const by=said(cover) || (from!==cover ? said(from) : null) || cover.tagName?.toLowerCase() || null;
    return {covered:true, by, controls, opens};
  }
  const popup=e.getAttribute('aria-haspopup'), expanded=e.getAttribute('aria-expanded');
  const announced=expanded!=='true' && ((!!popup && popup!=='false') || expanded==='false' ||
    !!e.getAttribute('aria-controls') || !!e.getAttribute('aria-owns'));
  return {...point, scrolled, announced, opens};
})
