import {Rig,boot,submit,sleep,runs,isTerminal,jq,recorder} from './lib.mjs';
const rec=recorder('l1');
const rig=new Rig('l1');await rig.start();await rig.session();await boot(rig);
const list=async()=>(await rig.send({ListQueuedWorkflowPrompts:{session_id:rig.sessionId}})).QueuedWorkflowPromptsListed.queued_prompts.map(x=>x.id);
const durable=()=>jq(rig,"select count(*) from durable_workflow_hot_entities where entity_kind='queued_prompt'")[0][0];
const plainSrc=`workflow.define({alias:'plain',maxConcurrent:32})\nconst n=workflow.node({handle:'entry',agent:workflow.newAgent({alias:'plain-entry',provider:'dev-stub',model:'default'}),instructions:'x',canCompleteWorkflowRun:true,maxTurns:1})\nworkflow.endpoint(n,{alias:'entry'})\n`;
await rig.apply(plainSrc);
const obs=[];
for(let i=0;i<4;i++){await rig.invoke('plain','entry','hello '+i);await sleep(1200);obs.push({path:'ungated',n:i+1,listed:(await list()).length,durable:durable()})}
// gated (first handoff starts the run, the rest queue behind it)
const d0=durable(),l0=(await list()).length;
for(let i=0;i<4;i++){const e=await submit(rig,'run');await rig.waitFor(rig.alias,e.entry_id,isTerminal,20000);await sleep(1200);obs.push({path:'gated',n:i+1,listed:(await list()).length-l0,durable:durable()-d0})}
// does an unrelated mutation refresh the live view?
await rig.newWorkflow('unrelated-mutation');await sleep(800);
const after={listed:(await list()).length-l0,durable:durable()-d0};
rec('listing-vs-durable',{result:'INFO',obs,afterUnrelatedMutation:after,expectedQueuedGated:3});
console.log(JSON.stringify(obs),JSON.stringify(after));
await rig.stop();
