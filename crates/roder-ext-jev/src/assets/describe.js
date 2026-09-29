// What the page shows besides its observation (Jev's own): the main
// document's HTTP status as the navigation reports it, and the visible
// headings (h1 to h3 and role=heading), at most 12 of 80 characters each.
(() => {
  const nav=performance.getEntriesByType('navigation')[0];
  const status=nav && nav.responseStatus>0 ? nav.responseStatus : null;
  const headings=[];
  for (const e of document.querySelectorAll('h1,h2,h3,[role="heading"]')) {
    if (headings.length>=12) break;
    if (!e.checkVisibility({checkOpacity:true,checkVisibilityCSS:true})) continue;
    const r=e.getBoundingClientRect();
    if (r.width<=0 || r.height<=0) continue;
    const text=(e.innerText||'').replace(/\s+/g,' ').trim().slice(0,80).toWellFormed();
    if (text && !headings.includes(text)) headings.push(text);
  }
  return {http_status:status,headings};
})()
