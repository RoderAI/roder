// Called on snapshot.js's result, after its freshness marker is built, so a
// context never makes a decision stale. Controls that read alike (the same
// role and label) each get a `context`: the card, row or section they sit in.
// Offscreen controls get their section too, since page text is viewport-only,
// and controls in a table get their column. The rules and their guards follow
// fastbrowse's snapshot.js (MIT), reimplemented here. Every control of the
// top document also gets a `section` (Jev's own): the nearest visible heading
// before it, for the result the caller reads. Neither the decision model nor
// the step fingerprint sees it.
(state => {
  if (!state) return state;
  const cache=window.__jevFast, visible=cache.visible, labelOf=cache.name;
  const CAP=120, GROUP_CAP=50;
  // Cut whole characters (never half an emoji's surrogate pair), well formed.
  const clip=s=>{
    if (s.length<=CAP) return s.toWellFormed();
    const t=s.slice(0,CAP-1);
    return (/[\uD800-\uDBFF]$/.test(t) ? t.slice(0,-1) : t).toWellFormed()+'…';
  };
  const squash=s=>(s||'').replace(/\s+/g,' ').trim();
  const firstLine=text=>(text||'').split('\n').map(squash).find(Boolean)||'';
  const baseLabel=a=>String(a.label).split(' → ')[0];
  const HEADINGS='h1,h2,h3,h4,h5,h6,[role="heading"],legend,caption,th,dt,summary';
  // A label tied to no control titles what follows it, like a heading. A
  // hidden title names nothing on screen.
  // Memoized per scope: offscreen controls share their ancestors, and a large
  // one (a grid of a thousand cards) is costly to scan.
  const scanned=new Map();
  const titles=scope=>{
    if (!scanned.has(scope)) scanned.set(scope,[...scope.querySelectorAll(HEADINGS+',label')]
      .filter(t=>(t.localName!=='label' || !t.control) && visible(t)));
    return scanned.get(scope);
  };
  // A header cell names only its own row and a caption only its own table:
  // a datepicker's Next button outside the grid is not "Su".
  const reaches=(title,e)=>!['th','caption'].includes(title.localName) ||
    title.parentElement.contains(e);
  // The nearest title before the element names its section, else the scope's
  // first title; only the first when `nearest` is false.
  const sectionOf=(scope,e,label,nearest=true)=>{
    let first='', before='';
    for (const title of titles(scope)) {
      const text=firstLine(title.innerText);
      if (!text || text===label || title.contains(e) || !reaches(title,e)) continue;
      first ||= text;
      if (title.compareDocumentPosition(e) & Node.DOCUMENT_POSITION_FOLLOWING) before=text;
    }
    return (nearest && before) || first;
  };
  const repeated=new Set();
  const nameOf=(scope,e,label,nearest,depth=1)=>{
    const aria=squash(scope.getAttribute('aria-label'));
    if (aria && aria!==label) return aria;
    const section=sectionOf(scope,e,label,nearest);
    if (section) return section;
    // A line that is itself a repeated label ("‹") names no repeat. The label
    // is cut out as whole words: a day "2" must not turn "October 2026" into
    // "October 0 6". A short row reads whole ("Bob Lunch"); otherwise its
    // first line, or its first `depth` lines.
    const lines=(scope.innerText||'').split('\n').map(squash)
      .filter(line=>line && !repeated.has(line))
      .map(line=>squash((' '+line+' ').split(' '+label+' ').join(' '))).filter(Boolean);
    const whole=lines.join(' ');
    return whole.length<=60 ? whole : lines.slice(0,depth).join(' ');
  };
  // The widest ancestor holding no other twin is the card or row the twins
  // repeat over; a narrower one would name the button's own wrapper.
  const contextOf=(e,twins,label,nearest=true,depth=1)=>{
    let scope=null;
    for (let p=e.parentElement; p && p!==document.body; p=p.parentElement) {
      if (twins.some(t=>t!==e && p.contains(t))) break;
      scope=p;
    }
    return scope ? nameOf(scope,e,label,nearest,depth) : '';
  };
  // Twins laid out together (in one table, or under one grandparent: a
  // board of row boxes, a CSS grid) that form a grid, each to its "row r,
  // column c".
  const gridsOf=twins=>{
    const sets=new Map(), found=new Map();
    for (const e of twins) {
      const key=e.closest('td,th')?.closest('table') || e.parentElement?.parentElement;
      if (!sets.has(key)) sets.set(key,[]);
      sets.get(key).push(e);
    }
    for (const set of sets.values()) {
      const names=set.length>=4 ? gridOf(set) : null;
      if (names) set.forEach((e,i)=>found.set(e,names[i]));
    }
    return found;
  };
  // Each twin's "row r, column c" when their centres fall on at least two
  // rows and two columns (within 4 px), else null.
  const gridOf=twins=>{
    const centres=twins.map(e=>{ const r=e.getBoundingClientRect(); return [r.x+r.width/2,r.y+r.height/2]; });
    const bands=values=>{
      const sorted=[...new Set(values)].sort((a,b)=>a-b), starts=[];
      for (const v of sorted) if (!starts.length || v-starts[starts.length-1].last>4) starts.push({first:v,last:v});
        else starts[starts.length-1].last=v;
      return v=>starts.findIndex(band=>v>=band.first && v<=band.last)+1;
    };
    const row=bands(centres.map(c=>c[1])), column=bands(centres.map(c=>c[0]));
    const rows=Math.max(...centres.map(c=>row(c[1]))), columns=Math.max(...centres.map(c=>column(c[0])));
    if (rows<2 || columns<2 || twins.length>rows*columns) return null;
    return centres.map(([x,y])=>'row '+row(y)+', column '+column(x));
  };
  const byPosition=(a,b)=>a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1;

  // Twins by role and label, each element once, nearest first.
  const groups=new Map(), elements=new Map();
  for (const a of state.actions) {
    if (a.node==null) continue;
    const e=cache.nodes.get(a.node);
    if (!e) continue;
    elements.set(a.node,e);
    const key=JSON.stringify([a.role,a.label]);
    if (!groups.has(key)) groups.set(key,{label:baseLabel(a),nodes:[]});
    const group=groups.get(key);
    if (!group.nodes.includes(a.node)) group.nodes.push(a.node);
  }
  for (const group of groups.values()) if (group.nodes.length>1) repeated.add(group.label);
  const context=new Map();
  for (const {label,nodes} of groups.values()) {
    if (nodes.length<2 || nodes.every(n=>context.has(n))) continue;
    const twins=nodes.map(n=>elements.get(n));
    // Only the nearest twins pay for a walk; the rest are placed by position.
    const named=twins.slice(0,GROUP_CAP);
    const nearest=named.map(e=>contextOf(e,twins,label));
    let names=named.map((e,i)=>
      // Cards that each end in the same "Details" heading would all read the
      // same; a twin sharing its nearest title takes its card's first title.
      nearest.indexOf(nearest[i])!==nearest.lastIndexOf(nearest[i]) ?
        contextOf(e,twins,label,false) : nearest[i]);
    // Rows that still read the same by their first line (two mails from
    // Bob) take their second and third lines too.
    for (let depth=2; depth<=3; depth++) {
      const clash=i=>!!names[i] && names.indexOf(names[i])!==names.lastIndexOf(names[i]);
      if (!names.some((_,i)=>clash(i))) break;
      names=names.map((name,i)=>clash(i) ? contextOf(named[i],twins,label,false,depth)||name : name);
    }
    // Twins whose surroundings name none of them, or too many to walk, still
    // differ by position: their row and column when they are laid out as a
    // grid (a board, a matrix of checkboxes), else their order.
    const ordered=[...twins].sort(byPosition);
    const grid=gridsOf(twins);
    nodes.forEach((n,i)=>{
      if (context.has(n)) return;
      const name=i<GROUP_CAP && names.some(Boolean) ? names[i] : grid.get(twins[i]) ||
        (ordered.indexOf(twins[i])+1)+' of '+twins.length;
      if (name) context.set(n,clip(name));
    });
  }
  // An offscreen control arrives without any page text around it, so it is
  // named by the nearest section that has a name, the body included.
  for (const a of state.actions) {
    if (!a.offscreen || context.has(a.node)) continue;
    const e=elements.get(a.node), label=baseLabel(a);
    for (let p=e?.parentElement; p; p=p===document.body ? null : p.parentElement) {
      const aria=squash(p.getAttribute('aria-label'));
      const name=aria && aria!==label ? aria : sectionOf(p,e,label);
      if (name) { context.set(a.node,clip(name)); break; }
    }
  }

  // Flattened text loses empty cells, so a calendar day loses its weekday.
  // Every control in a table keeps its column's header, honouring rowspan,
  // colspan and explicit `headers`.
  const tables=new Map();
  const columnsOf=table=>{
    const at=new Map(), headers=[], filled=[];
    [...table.rows].forEach((row,r)=>{
      let col=0;
      for (const cell of row.cells) {
        while (filled[col]>r) col++;
        const end=col+cell.colSpan;
        at.set(cell,[col,end,r]);
        // rowspan=0 runs to the end of the row's section.
        const left=row.parentElement.rows.length-row.sectionRowIndex;
        const span=Math.min(cell.rowSpan||left,left);
        for (let i=col; i<end; i++) filled[i]=r+span;
        if (cell.tagName==='TH' && !['row','rowgroup'].includes(cell.scope) &&
          (cell.scope || row.parentElement.tagName==='THEAD' ||
            (r===0 && [...row.cells].every(c=>c.tagName==='TH')))) headers.push(cell);
        col=end;
      }
    });
    return cell=>{
      const place=at.get(cell);
      if (!place) return '';
      const [start,end,row]=place;
      const associated=cell.getAttribute('headers') ?
        cell.getAttribute('headers').split(/\s+/).map(id=>document.getElementById(id)) :
        headers.filter(h=>{ const [l,r,above]=at.get(h); return above<row && l<end && r>start; });
      const names=associated.filter(h=>h && visible(h)).map(h=>squash(h.getAttribute('abbr') ||
        h.getAttribute('title') || h.querySelector('[title]')?.getAttribute('title') || labelOf(h)));
      return [...new Set(names.filter(Boolean))].join(' / ');
    };
  };
  const columns=new Map();
  for (const [n,e] of elements) {
    const cell=e.closest('td,th'), table=cell?.closest('table');
    if (!table) continue;
    if (!tables.has(table)) tables.set(table,columnsOf(table));
    const column=tables.get(table)(cell);
    if (column) columns.set(n,column);
  }
  for (const [n,column] of columns)
    context.set(n,clip([context.get(n),'column: '+column].filter(Boolean).join('; ')));

  for (const a of state.actions) if (context.has(a.node)) a.context=context.get(a.node);

  // Sections: the headings of the top document in document order, and for
  // each control the last one before it (by binary search) that does not
  // hold it and does not read as its label.
  const heads=[...document.querySelectorAll('h1,h2,h3,h4,[role="heading"],legend')]
    .filter(h=>visible(h)).map(h=>[h,clip(firstLine(h.innerText))]).filter(([,t])=>t);
  const before=(h,e)=>!!(h.compareDocumentPosition(e) & Node.DOCUMENT_POSITION_FOLLOWING);
  // A heading names what follows it in its own branch: the largest ancestor
  // of the heading that does not hold the control. When that branch holds a
  // neighbouring heading too, the heading titles one of several items (the
  // last card of a list) and the control sits after the list, not in it.
  const branchOf=(h,e)=>{
    let b=h;
    while (b.parentElement && !b.parentElement.contains(e)) b=b.parentElement;
    return b;
  };
  const titlesItem=(i,e)=>{
    const b=branchOf(heads[i][0],e);
    return [heads[i-1],heads[i+1]].some(n=>n && b!==heads[i][0] && b.contains(n[0]) &&
      !heads[i][0].contains(n[0]));
  };
  const sectionFor=e=>{
    let low=0, high=heads.length;
    while (low<high) { const mid=(low+high)>>1; if (before(heads[mid][0],e)) low=mid+1; else high=mid; }
    for (let i=low-1; i>=0 && i>=low-3; i--) {
      const [h,text]=heads[i];
      if (!h.contains(e) && !titlesItem(i,e)) return text;
    }
    return '';
  };
  const sections=new Map();
  if (heads.length) for (const a of state.actions) {
    const e=elements.get(a.node);
    if (!e || e.getRootNode()!==document) continue;
    if (!sections.has(a.node)) sections.set(a.node,sectionFor(e));
    const section=sections.get(a.node);
    if (section && section!==baseLabel(a)) a.section=section;
  }
  return state;
})
