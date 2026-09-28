// Resolve with the document's readyState once it is past `loading`, or
// after `timeoutMs` (jev-owned; replaces polling for `complete`).
(timeoutMs => new Promise(resolve => {
  if (document.readyState!=='loading') return resolve(document.readyState);
  const finish=()=>{
    clearTimeout(timer); document.removeEventListener('readystatechange',changed);
    resolve(document.readyState);
  };
  const changed=()=>{ if (document.readyState!=='loading') finish(); };
  const timer=setTimeout(finish,timeoutMs);
  document.addEventListener('readystatechange',changed);
}))
