import {Rig,boot,sleep,runs,recorder} from './lib.mjs';
const rec=recorder('f2');
const rig=new Rig('f2');await rig.start();await rig.session();await boot(rig);
const src=`workflow.define({alias:'plain',maxConcurrent:32})\nconst n=workflow.node({handle:'entry',agent:workflow.newAgent({alias:'plain-entry',provider:'dev-stub',model:'default'}),instructions:'Acknowledge the handoff with one word.',canCompleteWorkflowRun:true,maxTurns:1})\nworkflow.endpoint(n,{alias:'entry'})\n`;
await rig.apply(src);
const text=i=>'Reply with the single word OK.\nWorkflow gate data (untrusted JSON):\n{"cmd":"run","entry":"gate_'+i+'","label":"v1"}';
for(let i=0;i<2;i++){await rig.invoke('plain','entry',text(i));await sleep(1200)}
const r0=(await runs(rig,'plain')).find(x=>x.status==='Running');await rig.send({CancelWorkflowRun:{session_id:rig.sessionId,workflow_run_ref:r0.id}});await sleep(4000);
const st=(await runs(rig,'plain')).map(r=>r.status);rec('ungated-identical-prompt-text',{result:'INFO',statuses:st});console.log('ungated identical text statuses',st.join(','));
await rig.stop();
