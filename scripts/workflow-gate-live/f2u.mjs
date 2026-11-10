// f2g control: the same node graph without a gate. If the promoted run also fails after a cancel, the failure is not the gate's.
import {Rig,boot,sleep,runs,recorder} from './lib.mjs';
const rec=recorder('f2');
const rig=new Rig('f2u');await rig.start();await rig.session();await boot(rig,{config:null});
for(let i=0;i<3;i++){await rig.invoke(rig.alias,'entry','run');await sleep(1000)}
const log=[];const snap=async(t)=>log.push(t+': '+(await runs(rig)).map(r=>r.status).join(','));
await snap('after 3 ungated invokes');
for(let k=0;k<3;k++){const r=(await runs(rig)).find(x=>x.status==='Running');if(!r){log.push('no Running run at cancel '+(k+1));break}await rig.send({CancelWorkflowRun:{session_id:rig.sessionId,workflow_run_ref:r.id}});await sleep(3000);await snap('after cancel '+(k+1))}
rec('ungated-cancel-chain-same-graph',{result:'INFO',log});console.log(log.join(' | '));
await rig.stop();
