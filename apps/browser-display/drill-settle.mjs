// MD-DISPLAY-02: exact repair may require several credited tile batches.
export async function drainRepairs(next, limit=300) {
 let last=null;
 for(let polls=1;polls<=limit;polls++){
  const frame=await next();
  if(frame===null)return{polls,last};
  last=frame;
 }
 throw Error('MD-DISPLAY: bounded exact repair did not converge');
}
