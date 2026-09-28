// Wait inside the page until it has finished reacting (jev-owned; diverges
// from upstream's two-frame settle, and the loading-indicator list and the
// suggestion and popup rules follow fastbrowse's snapshot.js and page.py,
// MIT). Needs track.js first. Resolves once the document is past `loading`,
// the quiet clock has run for QUIET ms and no loading indicator shows in the
// viewport; indicators stop counting after `loadingMs`, and `capMs` bounds
// the whole wait. Two kinds of input promise more than quiet can show, so
// for up to EXPECT ms quiet is not enough: a fill into a field that offers
// suggestions (a combobox, a search field, aria-autocomplete, or one that
// controls or owns another element) returns as soon as it lists options
// other than those shown before the text was typed (see track.js),
// and a click on a control that announced a popup waits for it to expand or
// for the element it controls to show. Resolves with why it stopped and how
// long it waited.
(({action, capMs, loadingMs}) => new Promise(resolve => {
  const QUIET=200, POLL=50, EXPECT=1200;
  const state=window.__jevFast, start=performance.now();
  const field=action ? state.nodes.get(action.node) : null;
  const owned=()=>(field?.getAttribute('aria-controls')||field?.getAttribute('aria-owns')||'')
    .split(/\s+/).filter(Boolean).map(id=>document.getElementById(id)).filter(Boolean);
  const suggests=action?.kind==='fill' && !!field && (field.getAttribute('role')==='combobox' ||
    field.type==='search' || !!field.getAttribute('aria-autocomplete') || owned().length>0);
  const announced=action?.kind==='click' && !!action.announced && !!field;
  const LOADING='[aria-busy="true"],[role="progressbar"],progress:not([value]),' +
    ['spinner','loading','loader','skeleton','shimmer']
      .flatMap(name=>[`[class*="${name}" i]`,`[id*="${name}" i]`]).join(',');
  const showing=e=>{
    if (!e.checkVisibility({checkOpacity:true,checkVisibilityCSS:true})) return false;
    const r=e.getBoundingClientRect();
    return r.width>0 && r.height>0 && r.bottom>0 && r.right>0 && r.top<innerHeight && r.left<innerWidth;
  };
  // An open but empty list is what the wait is for, so only options count:
  // visible ones, and not the ones shown before the text was typed.
  const listed=()=>{
    const now=state.suggestions?.(field) ?? '';
    return now!=='' && now!==state.suggested;
  };
  const opened=()=>field.getAttribute('aria-expanded')==='true' ||
    owned().some(e=>e.checkVisibility({checkOpacity:true,checkVisibilityCSS:true}));
  // `due` is when the pending poll should run. One that runs a poll interval
  // late means the renderer stalled, and the page's own timers, queued before
  // it, may not have run yet: a quiet clock read then is not evidence of
  // quiet. Such a poll looks again once, straight after those timers; that
  // second look is trusted however late it runs, so steady load cannot keep
  // the settle from ever seeing quiet.
  let due=start, polled=false, recheck=false;
  const schedule=ms=>{ due=performance.now()+ms; setTimeout(poll,ms); };
  const done=(reason,waited)=>resolve({reason, waited_ms:Math.round(waited)});
  const poll=()=>{
    polled=true;
    const now=performance.now(), waited=now-start;
    const stalled=!recheck && now-due>POLL;
    recheck=false;
    if (suggests && listed()) return done('listbox',waited);
    const expecting=waited<EXPECT && (suggests || (announced && !opened()));
    const ready=document.readyState!=='loading';
    const quietFor=now-state.lastMutation;
    const quiet=ready && quietFor>=QUIET && !stalled && !expecting;
    if (quiet && (waited>=loadingMs || ![...document.querySelectorAll(LOADING)].some(showing)))
      return done('quiet',waited);
    if (waited>=capMs) return done('cap',waited);
    if (stalled) { recheck=true; return schedule(0); }
    // Aim at the moment quiet arrives; a quiet page with an indicator polls
    // at the plain interval, since each check walks the document.
    schedule(ready && quietFor<QUIET && !expecting ? Math.min(POLL, Math.max(1, QUIET-quietFor)) : POLL);
  };
  // After input, let one frame render first: a menu drawn in an animation
  // frame mutates only when that frame runs. The timer covers a tab that
  // produces no frames, so the cap is always reached. With no input pending,
  // look at once: an already quiet page costs nothing.
  const first=()=>{ if (!polled) { due=performance.now(); poll(); } };
  if (action && !document.hidden) {
    requestAnimationFrame(()=>setTimeout(first,0));
    setTimeout(first,100);
  } else first();
}))
