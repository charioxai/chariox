// MP-08 / MP-10 / MP-11: supplemental real-process signal regression, no provider acceptance.
import {writeFile} from 'node:fs/promises';
import {spawnOwned,signalOwnedProcessGroup} from './owned.mjs';
import {registerTerminationCleanup} from './termination.mjs';
const root=process.argv[2];if(!root?.startsWith(process.env.TMPDIR+'/'))throw Error('External test root required');
let child;
registerTerminationCleanup(async()=>{if(child){signalOwnedProcessGroup(child,'SIGTERM');for(let i=0;i<50&&child.exitCode===null&&child.signalCode===null;i++)await new Promise(r=>setTimeout(r,100));}await writeFile(root+'/cleaned','yes');});
if(process.argv[3]==='before-launch'){
 await writeFile(root+'/ready.json',JSON.stringify({driver:process.pid,resource:null}));
 setTimeout(()=>{child=spawnOwned(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore',detached:true});},10000);
}else{
 child=spawnOwned(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore',detached:true});
 await writeFile(root+'/ready.json',JSON.stringify({driver:process.pid,resource:child.pid}));
}
setInterval(()=>{},1000);
