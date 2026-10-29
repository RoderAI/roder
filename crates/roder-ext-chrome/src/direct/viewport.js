// Read complete visible text-node records before Rust scrubs and bounds them.
// A scroll changes this evidence even when document.body.innerText is unchanged.
(() => {
  const S = window.__roderDirect;
  S.viewportText = (wholeDocument = false) => {
    const words = [], links = new Set(); let size = 0, visited = 0, limited = false;
    const excluded = 'input,textarea,select,script,style,noscript,template,[contenteditable]';
    const read = root => {
      const doc = root.ownerDocument || document;
      const walker = doc.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
      const range = doc.createRange();
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        if (++visited > 20000) { limited = true; break; }
        if (node.nodeType === 1) {
          if (node.shadowRoot) read(node.shadowRoot);
          continue;
        }
        const parent = node.parentElement || node.parentNode?.host;
        let value = node.textContent.replace(/\s+/g, ' ').trim();
        if (!value || !parent || parent.closest(excluded)) continue;
        let left = 0, top = 0, right = innerWidth, bottom = innerHeight, hidden = false;
        for (let p = parent; p; p = p.parentElement || p.getRootNode()?.host) {
          const style = getComputedStyle(p), box = p.getBoundingClientRect();
          if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) { hidden = true; break; }
          if (!wholeDocument && ['hidden','clip','auto','scroll'].includes(style.overflowX)) { left = Math.max(left,box.left); right = Math.min(right,box.right); }
          if (!wholeDocument && ['hidden','clip','auto','scroll'].includes(style.overflowY)) { top = Math.max(top,box.top); bottom = Math.min(bottom,box.bottom); }
        }
        if (hidden || right <= left || bottom <= top) continue;
        range.selectNodeContents(node);
        if (![...range.getClientRects()].some(r => r.width > 0 && r.height > 0 && (wholeDocument || (r.right > left && r.left < right && r.bottom > top && r.top < bottom)))) continue;
        const link = wholeDocument && parent.closest('a[href]');
        if (link && /^https?:/.test(link.href)) {
          if (links.has(link)) continue;
          links.add(link); value = link.innerText.trim()+' ('+link.href+')';
        }
        // Never expose a partial record that could contain a truncated secret.
        if (size + value.length > 100000) { limited = true; continue; }
        words.push(value); size += value.length + 1;
      }
    };
    if (document.body) read(document.body);
    return {text: words.join('\n'), limited};
  };
})();
