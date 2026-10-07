// MD-DISPLAY-02: exact repair may require several credited tile batches.
export async function drainRepairs(next, limit=300, wait=ms=>new Promise(resolve=>setTimeout(resolve,ms))) {
 let last=null,quiet=false;
 for(let polls=1;polls<=limit;polls++){
  const frame=await next();
  if(frame===null) {
   if(quiet)return{polls,last};
   quiet=true;await wait(300);continue;
  }
  quiet=false;
  last=frame;
 }
 throw Error('MD-DISPLAY: bounded exact repair did not converge');
}
// Native controls can finish a compositor paint after an unchanged poll.
// Retry a bounded number of complete verification cycles; every accepted
// result still requires an independently captured source to match exactly.
export async function verifySettled(next,pair,attempts=3) {
 for(let attempt=1;attempt<=attempts;attempt++) {
  const repair=await drainRepairs(next),fidelity=await pair(attempt);
  if(fidelity.lossless)return {...repair,fidelity,verification_attempts:attempt};
 }
 throw Error('MD-DISPLAY: settled pixels differ after bounded verification');
}

// MD-DISPLAY-02/04: keep real continuous credits running through freeze/repair.
// Independent RGB readback binds the exact result to its actual presentation,
// rather than adding a manual poll pause to the measured restoration interval.
export async function verifyContinuousSettled(pair,{timeoutMs=90000,now=()=>performance.now(),wait=ms=>new Promise(resolve=>setTimeout(resolve,ms))}={}) {
 const started=now();
 for(let attempt=1;now()-started<timeoutMs;attempt++){
  const fidelity=await pair(attempt);
  if(fidelity.lossless&&Number.isFinite(fidelity.presentation_ms))return {fidelity,verification_attempts:attempt,polls:0};
  await wait(100);
 }
 throw Error('MD-DISPLAY: continuous exact repair did not converge');
}
