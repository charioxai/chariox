import {Rig,boot,submit,sleep,runs,recorder,isTerminal} from './lib.mjs';
const rec=recorder('f2');
const rig=new Rig('f2g');await rig.start();await rig.session();await boot(rig);
for(let i=0;i<3;i++){const e=await submit(rig,'run');await rig.waitFor(rig.alias,e.entry_id,isTerminal,20000);await sleep(1000)}
const log=[];const snap=async(t)=>log.push(t+': '+(await runs(rig)).map(r=>r.status).join(','));
await snap('after 3 gated handoffs');
for(let k=0;k<3;k++){const r=(await runs(rig)).find(x=>x.status==='Running');if(!r){log.push('no Running run at cancel '+(k+1));break}await rig.send({CancelWorkflowRun:{session_id:rig.sessionId,workflow_run_ref:r.id}});await sleep(3000);await snap('after cancel '+(k+1))}
rec('gated-cancel-chain-fixed-kernel',{result:'INFO',log});console.log(log.join(' | '));
await rig.stop();
