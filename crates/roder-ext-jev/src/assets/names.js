// How snapshot.js names what it offers (started from upstream Jev's
// snapshot.js, MIT; the rest is Jev's own). Called with snapshot.js's
// `visible` and `controls`; returns the naming helpers. A control is named
// by aria-labelledby, aria-label, its labels, a button's value, alt, its own
// text (never a field's: its content is its value), title or placeholder,
// then by the text around a field or an icon's picture.
(({visible, controls}) => {
  // Script and style text is never a name: a result card that nests a <style>
  // in its link would otherwise be labelled with CSS.
  const unnamed=['SCRIPT','STYLE','NOSCRIPT','TEMPLATE'];
  // aria-labelledby ids resolve in the element's own tree (a shadow root),
  // then in the document.
  const byId = (e,id) => (id && e.getRootNode().getElementById?.(id)) || document.getElementById(id);
  // A field is never named by its own content: a textarea's text, a select's
  // options, what an editable box or an ARIA textbox, searchbox or combobox
  // holds are its value. An ARIA 1.1 combobox wrapping its own input holds
  // that input, which names it.
  const fieldRoles=['textbox','searchbox','combobox'];
  const field = e => ['INPUT','TEXTAREA','SELECT'].includes(e.tagName) || e.isContentEditable ||
    (fieldRoles.includes(e.getAttribute('role')) && !e.querySelector('input,textarea,select'));
  // A field with no label, title or placeholder is named by the text around
  // it: the nearest of its first four ancestors that holds no other field and
  // some text outside controls, if that text reads as a label (one or two
  // runs of text, 60 characters at most). This finds a label with no "for",
  // a header cell in the field's row, and text before or after it in the
  // same box, but not a form's header, nor text outside a box that groups the
  // field with a button.
  const others='input:not([type=hidden],[type=submit],[type=button],[type=reset],[type=image]),'+
    'textarea,select,[contenteditable="true"]';
  const wordless='a,button,select,textarea,option,script,style,noscript,template,[role="button"],'+
    '[role="link"],[aria-hidden="true"]';
  const nearby = e => {
    let p=e.parentElement, branch=e;
    for (let depth=0; p && p!==document.body && depth<4; depth++, branch=p, p=p.parentElement) {
      if ([...p.querySelectorAll(others)].some(x=>x!==e && visible(x))) return '';
      // A field grouped with a button (a guess and its Submit) is not named
      // by text outside the group: that is more often a status line.
      if ([...branch.querySelectorAll(controls)].some(x=>x!==e && !e.contains(x) && visible(x))) return '';
      const words=[], walk=p.ownerDocument.createTreeWalker(p,NodeFilter.SHOW_TEXT);
      for (let t=walk.nextNode(); t; t=walk.nextNode()) {
        const value=t.textContent.trim();
        if (value && !e.contains(t) && !t.parentElement.closest(wordless) && visible(t.parentElement))
          words.push(value);
      }
      // A label is a phrase: one or two runs of text, not a header of lines.
      const text=words.join(' ').split(/\s+/).join(' ');
      if (text) return words.length<=2 && text.length<=60 ? text : '';
    }
    return '';
  };
  // An icon with no name is named by its picture's file: an img with no
  // alt, a CSS content or background image, or an SVG sprite, when the
  // file's stem reads as up to four words ("delete.png", "#icon-star").
  const stem=url=>{
    const file=String(url).split(/[?#]/)[0].split('/').pop()||'';
    const words=file.replace(/\.[a-z0-9]{2,5}$/i,'').replace(/[-_.\s]+/g,' ').trim();
    return /^[a-z]+(?: [a-z]+){0,3}$/i.test(words) && words.length<=30 ? words.toLowerCase() : '';
  };
  const picture=x=>{
    if (x.tagName==='IMG') return stem(x.currentSrc||x.src||'');
    if (x.tagName==='use') return stem((x.getAttribute('href')||x.getAttribute('xlink:href')||'').split('#').pop());
    const style=getComputedStyle(x);
    const drawn=style.content.startsWith('url(') ? style.content : style.backgroundImage;
    const url=/^url\(["']?([^"')]+)/.exec(drawn||'');
    return url && !url[1].startsWith('data:') ? stem(url[1]) : '';
  };
  const pictured=e=>{
    for (const x of [e,...[...e.querySelectorAll('img,use,span,i,div')].slice(0,4)]) {
      const words=picture(x);
      if (words) return words;
    }
    return '';
  };
  // What an element shows: a shadow host its shadow root, a slot its nodes.
  const kids = e => e.shadowRoot ? e.shadowRoot.childNodes :
    e.tagName==='SLOT' && e.assignedNodes().length ? e.assignedNodes({flatten:true}) : e.childNodes;
  const name = (e,seen=new Set()) => {
    if (!e || seen.has(e)) return '';
    seen.add(e);
    const top=seen.size===1;
    const referenced=(e.getAttribute('aria-labelledby')||'').split(/\s+/)
      .map(id=>name(byId(e,id),seen)).filter(Boolean).join(' ');
    return referenced || e.getAttribute('aria-label') ||
      [...(e.labels||[])].map(l=>name(l,seen)).filter(Boolean).join(' ') ||
      (['button','submit','reset'].includes(e.type) ? e.value : '') || e.getAttribute('alt') ||
      (field(e) ? '' : [...kids(e)].map(n=>n.nodeType===3 ? n.textContent :
        n.nodeType===1 && !unnamed.includes(n.tagName) && n.getAttribute('aria-hidden')!=='true' ?
          name(n,seen) : '').join(' ').trim()) ||
      e.getAttribute('title') || e.getAttribute('placeholder') || e.getAttribute('aria-placeholder') ||
      (top ? field(e) ? nearby(e) : pictured(e) : '');
  };
  const squash=s=>s.split(/\s+/).filter(Boolean).join(' ');
  // The first `n` UTF-16 units of `s`, never ending in half of a surrogate
  // pair (an emoji): Chrome would send that half as an unpaired escape.
  const cut=(s,n)=>{
    const t=s.slice(0,n);
    return /[\uD800-\uDBFF]$/.test(t) ? t.slice(0,-1) : t;
  };
  // A scroll box is named like a field when it is one, else by its own
  // label, its first heading or its first line.
  const regionName=e=>{
    if (field(e) || e.hasAttribute('aria-labelledby')) return name(e);
    const own=e.getAttribute('aria-label') || e.getAttribute('title');
    if (own) return own;
    const heading=e.querySelector('h1,h2,h3,h4,h5,h6,[role="heading"],legend,caption');
    const line=heading && visible(heading) ? squash(heading.innerText) :
      (e.innerText||'').split('\n').map(squash).find(Boolean)||'';
    return line.length>60 ? cut(line,59)+'…' : line;
  };
  return {name,field,squash,cut,regionName};
})
