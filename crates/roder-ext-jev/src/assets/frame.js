// Resolves once the page has presented a frame (jev-owned; follows
// fastbrowse's browser/page.py _PRESENTED_JS, MIT): two animation frames,
// then a task, so input is routed by the layout the DOM now describes and
// what a pointer move redrew is in place. Chrome routes input by the last
// drawn frame, not by the DOM. At once in a hidden tab, which draws no
// frames, and after 100 ms at most.
() => new Promise(done => {
  if (document.hidden) return done();
  const timer=setTimeout(done,100);
  requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(()=>{ clearTimeout(timer); done(); },0)));
})
