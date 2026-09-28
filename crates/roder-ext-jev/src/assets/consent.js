// Refuses a cookie or consent banner. Jev's own code, and the fallback for
// banners DuckDuckGo's autoconsent does not recognise: refuse.js calls it at
// most once per document, and only when autoconsent is not acting.
//
// Conservative on purpose: it acts only when exactly one visible dialog or
// banner region (a dialog role or element, a fixed or sticky box, or a box
// named for cookies or consent) holds text about cookies or consent and a
// button whose whole label is a refusal ("Reject all", "Only necessary",
// "Alle ablehnen"...). It clicks that button and nothing else: never an
// accept, agree or settings button, never outside the region, and nothing
// when two regions qualify or the best refusal label is on two buttons.
// Returns {clicked: label} or {skipped: why}.
(() => {
  const MAX_TEXT = 4000, MAX_CONTROLS = 30;
  const TOPIC = /cookie|consent|gdpr|datenschutz|einwilligung|consentement|consentimiento|consenso|toestemming|consentimento|zgod|samtycke|samtykke/i;
  const NAMED = /cookie|consent|gdpr|\bcmp\b/i;
  // Whole labels, lower case, spaces collapsed, end punctuation dropped;
  // best first.
  const REFUSE = [
    /^(reject|decline|refuse|deny) all( cookies)?$/,
    /^(reject|decline|refuse|deny)( (optional|non-essential|additional|all non-essential) cookies| cookies)?$/,
    /^(use )?(only|strictly) (necessary|essential|required)( cookies)?( only)?$/,
    /^(necessary|essential|required|strictly necessary) (cookies )?only$/,
    /^continue without accepting$/,
    /^(alle ablehnen|ablehnen|alle cookies ablehnen|nur (notwendige|erforderliche|essenzielle)( cookies)?)$/,
    /^(tout refuser|refuser tout|refuser|continuer sans accepter|refuser les cookies)$/,
    /^(rechazar todo|rechazar todas|rechazar|solo (necesarias|esenciales)|rechazar cookies)$/,
    /^(rifiuta tutto|rifiuta tutti|rifiuta|solo (necessari|essenziali))$/,
    /^(alles weigeren|alle weigeren|weigeren|alleen (noodzakelijk|noodzakelijke cookies))$/,
    /^(rejeitar tudo|rejeitar todos|rejeitar|recusar|recusar tudo|apenas (necessários|essenciais))$/,
  ];
  const ACCEPTS = /\b(accept|agree|allow|consent|ok|okay|got it|yes|akzeptieren|zustimmen|erlauben|accepter|j'accepte|aceptar|acepto|accetta|accetto|accepteren|akkoord|aceitar|aceito)\b/;
  const SETTINGS = /\b(manage|settings|preferences|customi[sz]e|options|more|details|learn|einstellungen|anpassen|paramètres|personnaliser|configurar|gestisci|personalizza|instellingen|aanpassen|definições)\b/;
  const clean = s => String(s||'').replace(/\s+/g,' ').trim().toLowerCase().replace(/[.!…]+$/,'');
  const visible = e => {
    if (!e.checkVisibility?.({checkOpacity:true,checkVisibilityCSS:true})) return false;
    const r=e.getBoundingClientRect();
    return r.width>1 && r.height>1 && r.bottom>0 && r.right>0 && r.top<innerHeight && r.left<innerWidth;
  };
  const label = e => clean(e.tagName==='INPUT' ? e.value : e.innerText || e.getAttribute('aria-label'));
  const refusal = text => {
    if (!text || ACCEPTS.test(text.replace('sans accepter','')) || SETTINGS.test(text)) return -1;
    return REFUSE.findIndex(re => re.test(text));
  };
  // Open shadow roots too: some consent platforms draw in one.
  const roots=[document];
  for (let i=0;i<roots.length;i++)
    for (const e of roots[i].querySelectorAll('*')) if (e.shadowRoot) roots.push(e.shadowRoot);
  const up = e => e.parentElement || e.getRootNode()?.host || null;
  const isRegion = e => {
    if (e===document.body || e===document.documentElement) return false;
    const role=e.getAttribute('role');
    if (role==='dialog' || role==='alertdialog' || e.tagName==='DIALOG' || e.getAttribute('aria-modal')==='true') return true;
    if (NAMED.test(e.id||'') || NAMED.test(typeof e.className==='string' ? e.className : '')) return true;
    const pos=getComputedStyle(e).position;
    return pos==='fixed' || pos==='sticky';
  };
  // The nearest region around each text that mentions the topic.
  const regions=new Set();
  for (const root of roots) {
    const walk=document.createTreeWalker(root,NodeFilter.SHOW_TEXT);
    for (let t=walk.nextNode(); t; t=walk.nextNode()) {
      if (!TOPIC.test(t.data)) continue;
      for (let e=t.parentElement; e; e=up(e)) if (isRegion(e)) { regions.add(e); break; }
    }
  }
  const CONTROL='button,[role="button"],a,input[type="button"],input[type="submit"]';
  // The region's enabled, visible buttons, through open shadow roots. A
  // link that goes somewhere is not a button.
  const controls = region => {
    const found=[], stack=[region];
    while (stack.length) {
      const node=stack.pop();
      for (const e of [...(node.shadowRoot?.children || []), ...node.children]) {
        if (e.matches(CONTROL)) found.push(e);
        stack.push(e);
      }
    }
    return found.filter(e => {
      const href=e.getAttribute('href');
      if (e.tagName==='A' && href!==null && e.getAttribute('role')!=='button' &&
          !/^(#|javascript:)/i.test(href)) return false;
      return !e.disabled && e.getAttribute('aria-disabled')!=='true' && visible(e);
    });
  };
  const qualified=[];
  for (const region of regions) {
    if (!visible(region)) continue;
    const text=region.innerText || region.textContent || '';
    if (!TOPIC.test(text) || text.length>MAX_TEXT) continue;
    const found=controls(region);
    if (found.length>MAX_CONTROLS) continue;
    let best=-1, buttons=[];
    for (const e of found) {
      const rank=refusal(label(e));
      if (rank<0) continue;
      if (best<0 || rank<best) { best=rank; buttons=[e]; }
      else if (rank===best) buttons.push(e);
    }
    if (best>=0) qualified.push({region,buttons});
  }
  // A region inside another qualifying one is the banner; the outer one is
  // its backdrop or page shell.
  const contains = (outer,inner) => { for (let e=inner; e; e=up(e)) if (e===outer) return true; return false; };
  const innermost=qualified.filter(q => !qualified.some(o => o!==q && contains(q.region,o.region)));
  if (!innermost.length) return {skipped:'no banner'};
  if (innermost.length>1) return {skipped:'more than one banner'};
  const {buttons}=innermost[0];
  const labels=new Set(buttons.map(label));
  if (buttons.length!==1 || labels.size!==1) return {skipped:'more than one refusal button'};
  const button=buttons[0];
  const shown=String((button.tagName==='INPUT' ? button.value : button.innerText) ||
    button.getAttribute('aria-label') || '').replace(/\s+/g,' ').trim().slice(0,80);
  button.click();
  return {clicked:shown};
})
