// Choose an observed option of a native select, after act.js has hit-tested
// it (jev-owned). Returns null when the select or the option is gone, else
// whether the page kept the choice and the options it shows: a change
// handler can refuse a choice by putting the old one back. The refusal check
// follows fastbrowse's browser/page.py (MIT).
(action => {
  const e=window.__jevFast?.nodes.get(action.node);
  if (e?.tagName!=='SELECT' || ![...e.options].some(o=>o.value===action.value &&
      !o.disabled && !o.closest('optgroup[disabled]'))) return null;
  e.value=action.value;
  e.dispatchEvent(new Event('input',{bubbles:true}));
  e.dispatchEvent(new Event('change',{bubbles:true}));
  return {kept:e.value===action.value, shown:[...e.selectedOptions].map(o=>o.label).join(', ')};
})
