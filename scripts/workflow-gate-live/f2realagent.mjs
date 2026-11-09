// Cancel chain with a real Codex Room agent (gpt-6-luna low): the agent starts the workflow through its own
// `chariox.room.run_command` tool, and the owner cancels the agent's task with CancelAgentTask while the node runs.
// One real model turn per run (the node is the dev-stub unless NODE_PROVIDER=codex). F2_UNGATED=1 drops the gate.
import {Rig,boot,sleep,recorder,isTerminal,runs,handoffs,wfSource} from './lib.mjs';
import {rmSync,mkdirSync} from 'node:fs';
const TREE=new URL('../../',import.meta.url).pathname.replace(/\/$/,'');
const requests=await import(TREE+'/packages/kernel-client/dist/ipc-requests.js');
const GATED=process.env.F2_UNGATED!=='1';const NODE_CODEX=process.env.NODE_PROVIDER==='codex';
const rec=recorder('f2realagent');const T0=Date.now();const say=(...a)=>console.log(((Date.now()-T0)/1000).toFixed(1).padStart(6)+'s',...a);
const CODEX_SRC='/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex';
const rig=new Rig('f2realagent',{extraEnv:{CHARIOX_CODEX_BIN:'/usr/local/bin/codex',IS_SANDBOX:'1',CHARIOX_ROOM_AGENT_TOOLS:'1'}});
rmSync(rig.dir,{recursive:true,force:true});await rig.start();
const prof=(await rig.send({LinkProviderAccountProfile:{provider:'codex',label:'wfreal-luna',path:CODEX_SRC}})).ProviderAccountProfile.profile.profile_id;
await rig.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:prof}});
const ws=rig.dir+'/ws';mkdirSync(ws,{recursive:true});
const made=(await rig.send({CreateSession:{workspace_id:ws,worktree_id:ws,alias:'f2ra',slice_ref:null,agent_defaults:{provider:'codex',account_profile:prof,model:'gpt-6-luna',effort:'low',permission_level:'yolo'}}})).SessionCreated;
rig.sessionId=made.session.id;const agentId=made.agent.id;
const att=(await rig.send({AttachToSession:{session_id:rig.sessionId,client_id:'f2ra',capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
const node=NODE_CODEX?{provider:'codex',model:'gpt-6-luna',agentExtra:{effort:'low',accountProfile:prof}}:{};
if(GATED)await boot(rig,node);
else{await rig.newWorkflow('ctl');await rig.apply(wfSource({alias:'gw',module:null,config:null,...node}));rig.alias='gw'}
const A=rig.alias;
const codexKids=()=>rig.kids().filter(k=>/codex/i.test(k.comm)).map(k=>k.pid+':'+k.stat);
const st=async()=>(await rig.send({GetSessionState:{session_id:rig.sessionId}})).SessionState.session;
const snapRuns=async()=>(await runs(rig)).map(r=>r.status+'['+(r.node_runs??[]).map(n=>n.status+'/'+(n.turn_envelope?.state??'-')).join(',')+']');
await rig.send(requests.submitPromptRequest(rig.sessionId,att,agentId,`Call the chariox room run_command tool exactly once with the command \`workflow run ${A} entry run\`. Do not run anything else. Then reply with one word: started.`,[]));
say('prompt submitted');
let task,seen=false,last='';const t=Date.now();
for(;;){const s=await snapRuns();const s0=await st();const tasks=(s0.agent_tasks??[]).filter(x=>x.agent_id===agentId);
  const j=JSON.stringify([s,tasks.map(x=>x.state+'/r'+x.revision+'/'+(x.obligations??[]).map(o=>o.kind+':'+o.status).join('+'))]);if(j!==last){say('state',j,'codex',JSON.stringify(codexKids()));last=j}
  task=tasks.find(x=>JSON.stringify(x).includes('gate_')||JSON.stringify(x).includes('workflow_run'));
  if(task&&s.some(x=>x.includes('Running/'))){seen=true;break}if(Date.now()-t>120000)break;await sleep(300)}
if(!task){say('no workflow task');rec('real-agent-cancel-'+(GATED?'gated':'ungated'),{result:'NOT RUN',why:'agent never created a workflow obligation'});await rig.stop();process.exit(4)}
const pre={seen,task:{id:task.task_id,state:task.state,rev:task.revision},entries:GATED?(await rig.allEntries(A)).map(e=>e.disposition):'ungated',runs:await snapRuns(),codex:codexKids(),active:(await st()).active_prompt?.status??null};
say('PRE',JSON.stringify(pre));
const tc=Date.now();let cancel='pending',answerMs=null;
rig.send({CancelAgentTask:{session_id:rig.sessionId,attachment_id:att,task_id:task.task_id,revision:task.revision}}).then(r=>{answerMs=Date.now()-tc;cancel='ANSWER '+JSON.stringify(r).slice(0,160);say('cancel answered after',answerMs,'ms')},e=>{answerMs=Date.now()-tc;cancel='ERR '+String(e).slice(0,300);say('cancel error after',answerMs,'ms',cancel)});
let post;const marks={};for(let i=0;i<240;i++){await sleep(500);const el=Date.now()-tc;const sx=await st();const ents=GATED?await rig.allEntries(A):[];
  post={cancel,answerMs,afterMs:el,tasks:(sx.agent_tasks??[]).filter(x=>x.agent_id===agentId).map(x=>x.state),entries:ents.map(e=>e.disposition+'/'+(e.failure??'')),runs:await snapRuns(),codex:codexKids(),handoffs:await handoffs(rig),orphanDeferred:ents.filter(e=>!isTerminal(e)).length,active:sx.active_prompt?.status??null};
  for(const m of [5000,30000,90000])if(el>=m&&!marks[m]){marks[m]=post.codex;say('codex at +'+m/1000+'s',JSON.stringify(post.codex))}
  if(cancel!=='pending'&&el>=(+process.env.HOLD_MS||8000))break}
say('POST',JSON.stringify(post));
const ok=answerMs!==null&&answerMs<=2000&&post.orphanDeferred===0&&post.handoffs<=1&&post.runs.every(r=>/^(Stopped|Completed|Failed|Cancelled)/.test(r))&&post.tasks.every(x=>/cancel|complet|failed|stopped/i.test(x));
rec('real-agent-cancel-'+(NODE_CODEX?'codexnode-':'stubnode-')+(GATED?'gated':'ungated'),{result:ok?'PASS':'FAIL',pre,post,marks});
await rig.stop();process.exit(ok?0:3);
