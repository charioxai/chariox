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
