// Agent cancelled while its workflow task is a gated entry (RED 2 lookup): a dev-stub runtime-tool agent
// starts the gated workflow through `workflow run`, then its task is cancelled.
import {Rig,boot,submit,sleep,recorder,isTerminal,jq,runs,handoffs,settled,kinds} from './lib.mjs';
const TREE=new URL('../../',import.meta.url).pathname.replace(/\/$/,'');
const requests=await import(TREE+'/packages/kernel-client/dist/ipc-requests.js');
const {bindProviderMcpFixture}=await import(TREE+'/apps/cli/scripts/lib/provider-mcp-fixture.mjs');
const rec=recorder('f2agent');const prompt=process.env.F2_PROMPT||'defer:120';
const rig=new Rig('f2agent');await rig.start();
const made=(await rig.send({CreateSession:{workspace_id:rig.dir+'/../ws',worktree_id:rig.dir+'/../ws',alias:'f2a',slice_ref:null,agent_defaults:{provider:'dev-stub',account_profile:'default',model:'runtime-mcp-fixture',permission_level:'yolo'}}})).SessionCreated;
rig.sessionId=made.session.id;const agentId=made.agent.id;
const att=(await rig.send({AttachToSession:{session_id:rig.sessionId,client_id:'f2a',capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
await boot(rig);const A=rig.alias;
const l=await rig.send(requests.launchProviderRunsRequest([{sessionId:rig.sessionId,provider:'dev-stub',accountProfile:'default',effort:'low',model:'runtime-mcp-fixture',agentId,native:{nativeTui:true}}],1));
const runId=l.ProviderRunsLaunchAccepted.provider_runs[0].provider_run.id;
let run;for(let i=0;i<120;i++){const r=Object.values(await rig.send(requests.getProviderRunRequest(runId)))[0];const p=r.provider_run??r;if(p.state==='Running'){run=p;break}await sleep(500)}
if(!run)throw Error('fixture run not running');
const fx=bindProviderMcpFixture(run,{client:rig.client,requests,sessionId:rig.sessionId,attachmentId:att});
// A 'go' sent before the fixture's READY banner settles on the banner alone and leaves no active task.
const runningAt=Date.now();let banner='';
while(!banner.includes('MP11_MCP_FIXTURE_READY')&&Date.now()-runningAt<1000){const o=await rig.send(requests.pumpTerminalOutputRequest(rig.sessionId,att));for(const r of (o.TerminalOutput??o.TerminalOutputPumped)?.records??[])if(r.provider_run_id===runId)banner+=Array.isArray(r.bytes)?Buffer.from(r.bytes).toString('utf8'):String(r.text??'');await sleep(50)}
console.log('FIXTURE_READY',banner.includes('MP11_MCP_FIXTURE_READY')?'banner':'running-1s',Date.now()-runningAt,'ms');
const sp=await rig.send(requests.submitPromptRequest(rig.sessionId,att,agentId,'go',[]));
await sleep(1500);
const tool=async(name,args)=>{const res=await fx.fixtureMcp.call('tools/call',{name,arguments:args},{timeoutMs:40000,rawEnvelope:true});return res.error?{rpcError:res.error}:(res.result??res)};
const st=async()=>(await rig.send({GetSessionState:{session_id:rig.sessionId}})).SessionState.session;
const busy=process.env.F2_BUSY==='1';
if(busy){const e=await submit(rig,'run');await rig.waitFor(A,e.entry_id,isTerminal,30000);for(let i=0;i<30&&!(await runs(rig)).some(r=>r.status==='Running');i++)await sleep(500)}
const out=await tool('chariox.room.run_command',{command:`workflow run ${A} entry ${prompt}`});
const txt=JSON.stringify(out).slice(0,400);
await sleep(2500);
let s=await st();const tasks=(s.agent_tasks??[]).filter(t=>t.agent_id===agentId);
const ents0=await rig.allEntries(A);const q0=(await st()).queued_workflow_prompts;
const pre={cmd:txt,q0:q0?.length??q0,tasks:tasks.map(t=>({id:t.task_id,status:t.status,rev:t.revision,res:JSON.stringify(t).match(/gate_[0-9a-f]+/g)?.slice(0,2)})),entries:ents0.map(e=>e.entry_id.slice(0,12)+':'+e.disposition)};
console.log('PRE',JSON.stringify(pre));
const task=tasks.find(t=>JSON.stringify(t).includes('gate_'))??tasks[0];
const qpB=jq(rig,"select json_extract(payload_json,'$.status'),count(*) from durable_workflow_hot_entities where entity_kind='queued_prompt' group by 1");console.log('QP_BEFORE',JSON.stringify(qpB));
let cancel;try{cancel=await rig.send({CancelAgentTask:{session_id:rig.sessionId,attachment_id:att,task_id:task.task_id,revision:task.revision}});cancel=JSON.stringify(cancel).slice(0,300)}catch(e){cancel='ERR '+String(e).slice(0,300)}
await sleep(4000);
s=await st();const ents=await rig.allEntries(A);const rs=await runs(rig);
const post={cancel,tasks:(s.agent_tasks??[]).filter(t=>t.agent_id===agentId).map(t=>({id:t.task_id,status:t.status})),entries:ents.map(e=>e.entry_id.slice(0,12)+':'+e.disposition+'/'+(e.failure??'')),runs:rs.map(r=>r.status),handoffs:await handoffs(rig),qp:jq(rig,"select json_extract(payload_json,'$.status'),count(*) from durable_workflow_hot_entities where entity_kind='queued_prompt' group by 1"),qpBefore:qpB,orphanDeferred:ents.filter(e=>!isTerminal(e)).length,queued:(s.queued_workflow_prompts??'n/a'),taskKeys:Object.keys(task),taskNow:JSON.stringify((s.agent_tasks??[]).find(t=>t.task_id===task.task_id)).slice(0,500)};
console.log('POST',JSON.stringify(post));
const agentEnts=ents.filter(e=>!busy||e.entry_id===ents.at(-1).entry_id);const last=ents.at(-1);
const ok=isTerminal(last)&&post.orphanDeferred===0&&(busy?(last.disposition==='cancelled'||last.disposition==='queued'):last.disposition==='cancelled')&&(busy?post.handoffs<=2:post.handoffs===0);
rec('agent-cancel-gated-'+prompt+(busy?'-busy':''),{result:ok?'PASS':'FAIL',pre,post});
await rig.stop();process.exit(ok?0:3);
