// Real-node cancel chain: a runtime-tool fixture agent (script-driven) starts the gated workflow whose node is a real
// Codex agent (gpt-6-luna low); the agent's task is cancelled while the node turn runs. One real model turn.
import {Rig,boot,sleep,recorder,isTerminal,jq,runs,handoffs} from './lib.mjs';
import {rmSync} from 'node:fs';
const TREE=new URL('../../',import.meta.url).pathname.replace(/\/$/,'');
const requests=await import(TREE+'/packages/kernel-client/dist/ipc-requests.js');
const {bindProviderMcpFixture}=await import(TREE+'/apps/cli/scripts/lib/provider-mcp-fixture.mjs');
const rec=recorder('f2realcancel');const T0=Date.now();const say=(...a)=>console.log(((Date.now()-T0)/1000).toFixed(1).padStart(6)+'s',...a);
const GATED=process.env.F2_UNGATED!=='1';const WAITPOST=+(process.env.WAITPOST||150);const AT=process.env.CANCEL_AT||'Running';
const CODEX_SRC='/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex';
const rig=new Rig('f2realcancel',{extraEnv:{CHARIOX_CODEX_BIN:'/usr/local/bin/codex',IS_SANDBOX:'1'}});
rmSync(rig.dir,{recursive:true,force:true});await rig.start();
const prof0=(await rig.send({LinkProviderAccountProfile:{provider:'codex',label:'wfreal-luna',path:CODEX_SRC}})).ProviderAccountProfile.profile.profile_id;
const prof=prof0;await rig.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:prof}});
const ws=rig.dir+'/ws';(await import('node:fs')).mkdirSync(ws,{recursive:true});
const made=(await rig.send({CreateSession:{workspace_id:ws,worktree_id:ws,alias:'f2rc',slice_ref:null,agent_defaults:{provider:'dev-stub',account_profile:'default',model:'runtime-mcp-fixture',permission_level:'yolo'}}})).SessionCreated;
rig.sessionId=made.session.id;const agentId=made.agent.id;
const att=(await rig.send({AttachToSession:{session_id:rig.sessionId,client_id:'f2rc',capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
const STUB=process.env.NODE_PROVIDER==='dev-stub';
if(GATED)await boot(rig,STUB?{}:{provider:'codex',model:'gpt-6-luna',agentExtra:{effort:'low',accountProfile:prof}});
else{const {wfSource}=await import('./lib.mjs');await rig.newWorkflow('ctl');await rig.apply(STUB?wfSource({alias:'gw',module:null,config:null}):wfSource({alias:'gw',module:null,config:null,provider:'codex',model:'gpt-6-luna',agentExtra:{effort:'low',accountProfile:prof}}));rig.alias='gw'}
const A=rig.alias;
const l=await rig.send(requests.launchProviderRunsRequest([{sessionId:rig.sessionId,provider:'dev-stub',accountProfile:'default',effort:'low',model:'runtime-mcp-fixture',agentId,native:{nativeTui:true}}],1));
const runId=l.ProviderRunsLaunchAccepted.provider_runs[0].provider_run.id;
let run;for(let i=0;i<120;i++){const r=Object.values(await rig.send(requests.getProviderRunRequest(runId)))[0];const p=r.provider_run??r;if(p.state==='Running'){run=p;break}await sleep(500)}
if(!run)throw Error('fixture run not running');
const fx=bindProviderMcpFixture(run,{client:rig.client,requests,sessionId:rig.sessionId,attachmentId:att});
let toolDone=false;
const st=async()=>(await rig.send({GetSessionState:{session_id:rig.sessionId}})).SessionState.session;
const codexKids=()=>rig.kids().filter(k=>/codex/i.test(k.comm)).map(k=>k.pid+':'+k.comm);
let toolP;for(let attempt=1;attempt<=4&&!toolDone;attempt++){await rig.send(requests.submitPromptRequest(rig.sessionId,att,agentId,'go',[]));await sleep(400);
  toolP=fx.fixtureMcp.call('tools/call',{name:'chariox.room.run_command',arguments:{command:`workflow run ${A} entry run`}},{timeoutMs:240000,rawEnvelope:true}).then(res=>{toolDone=true;say('TOOL RESULT',attempt,JSON.stringify(res.error?res.error:res.result??res).slice(0,160))},e=>say('TOOL ERR',String(e).slice(0,200)));
  for(let i=0;i<30&&!toolDone;i++)await sleep(300);if(!toolDone)say('tool attempt',attempt,'no answer in 9s')}
const snapRuns=async()=>(await runs(rig)).map(r=>r.status+'['+(r.node_runs??[]).map(n=>n.status+'/'+(n.turn_envelope?.state??'-')).join(',')+']');
let t=Date.now(),last='',seen=false;for(;;){const s=await snapRuns();const j=JSON.stringify(s);if(j!==last){say('runs',j,'codex',JSON.stringify(codexKids()));last=j}
  if(s.some(x=>x.includes(AT==='Running'?'Running/':'acknowledged'))){seen=true;break}if(Date.now()-t>60000)break;await sleep(300)}
const s0=await st();const tasks=(s0.agent_tasks??[]).filter(t=>t.agent_id===agentId);say('TASKS',JSON.stringify(tasks).slice(0,900));const task=tasks.find(t=>JSON.stringify(t).includes('gate_')||JSON.stringify(t).includes('workflow_run'))??tasks.at(-1);
const pre={seen,tasks:tasks.map(t=>({id:t.task_id,status:t.status,rev:t.revision})),entries:GATED?(await rig.allEntries(A)).map(e=>e.disposition):'ungated',runs:await snapRuns(),codex:codexKids()};
say('PRE',JSON.stringify(pre));
const tc=Date.now();let cancel='pending';const cancelP=rig.send({CancelAgentTask:{session_id:rig.sessionId,attachment_id:att,task_id:task.task_id,revision:task.revision}}).then(r=>{cancel='ANSWER '+JSON.stringify(r).slice(0,200);say('cancel answered after',Date.now()-tc,'ms',cancel)},e=>{cancel='ERR '+String(e).slice(0,300);say('cancel error after',Date.now()-tc,'ms',cancel)});
say('cancel',cancel);
if(process.env.STACK){await sleep(6000);const {spawnSync}=await import('node:child_process');const o=spawnSync('eu-stack',['-p',String(rig.pid),'-m','-n','40'],{encoding:'utf8',timeout:90000,maxBuffer:1<<26});(await import('node:fs')).writeFileSync(process.env.STACK,(o.stdout||'')+(o.stderr||''));say('stack written',(o.stdout||'').length,'cancel',cancel)}
let post;for(let i=0;i<WAITPOST;i++){await sleep(500);const sx=await st();const ents=GATED?await rig.allEntries(A):[];const rs=await runs(rig);
  post={cancel,afterMs:Date.now()-tc,tasks:(sx.agent_tasks??[]).filter(t=>t.agent_id===agentId).map(t=>t.status),entries:ents.map(e=>e.disposition+'/'+(e.failure??'')),runs:await snapRuns(),codex:codexKids(),handoffs:GATED?await handoffs(rig):rs.length,orphanDeferred:ents.filter(e=>!isTerminal(e)).length,qp:jq(rig,"select json_extract(payload_json,'$.status'),count(*) from durable_workflow_hot_entities where entity_kind='queued_prompt' group by 1")};
  const settled=cancel!=='pending'&&rs.every(r=>['Completed','Failed','Stopped','Cancelled'].includes(r.status))&&post.orphanDeferred===0&&post.codex.length===0;if(settled&&i>2)break}
const sf=await st();const fr=Object.values(await rig.send(requests.getProviderRunRequest(runId)))[0];post.fixture={activePrompt:sf.active_prompt?{status:sf.active_prompt.status,target:sf.active_prompt.target_agent_id===agentId?'fixture':'other'}:null,fixtureRunState:(fr.provider_run??fr).state};
say('POST',JSON.stringify(post));
const lastEnt=GATED?(await rig.allEntries(A)).at(-1):null;
const ok=post.orphanDeferred===0&&post.codex.length===0&&post.handoffs<=1&&post.tasks.every(x=>/cancel|complet|failed|stopped/i.test(x));
rec('cancel-chain-'+(STUB?'stubnode-':'')+(GATED?'gated':'ungated')+'-at-'+AT,{result:ok?'PASS':'FAIL',pre,post,entry:lastEnt&&{d:lastEnt.disposition,f:lastEnt.failure}});
await Promise.race([toolP,sleep(3000)]);await rig.stop();process.exit(ok?0:3);
