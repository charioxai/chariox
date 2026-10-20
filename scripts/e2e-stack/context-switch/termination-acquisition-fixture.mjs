// MP-08 / MP-10 / MP-11: interrupt while a resource reservation is still pending.
import {writeFile,rm} from 'node:fs/promises';
import {acquireTerminationResource,registerTerminationCleanup} from './termination.mjs';
const root=process.argv[2];if(!root?.startsWith(process.env.TMPDIR+'/'))throw Error('External test root required');
let settled=false;registerTerminationCleanup(()=>writeFile(root+'/cleaned',settled?'yes':'early'));
await writeFile(root+'/ready.json',JSON.stringify({driver:process.pid}));
const reserve=async()=>{await new Promise(r=>setTimeout(r,1500));await writeFile(root+'/reservation','owned',{flag:'wx'});settled=true;};
if(process.env.CTXVAL_TEST_UNTRACKED_RESERVATION==='1')await reserve();else await acquireTerminationResource(reserve,()=>rm(root+'/reservation',{force:true}));
setInterval(()=>{},1000);
