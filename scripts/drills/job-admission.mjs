// Real runtime MCP admission, shared unread-output and reader-lease drill on the named remote host.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdirSync,writeFileSync,openSync,rmSync,readFileSync} from 'node:fs';
import {createServer} from 'node:net';
import {hostname} from 'node:os';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const tree=fileURLToPath(new URL('../../',import.meta.url)).replace(/\/$/,'');
const {LocalIpcClient}=await import(tree+'/packages/kernel-client/dist/ipc.js');
const requests=await import(tree+'/packages/kernel-client/dist/ipc-requests.js');
const {bindProviderMcpFixture}=await import(tree+'/apps/cli/scripts/lib/provider-mcp-fixture.mjs');
const flags=new Map();for(let i=2;i<process.argv.length;i+=2)flags.set(process.argv[i],process.argv[i+1]);
assert(hostname()===flags.get('--load-host'),'Run on the named dedicated drill host');
const binary=flags.get('--kernel-binary'), label=flags.get('--label'), mode=flags.get('--mode')??'head';
const root=flags.get('--state-root'), evidence=flags.get('--evidence-dir');
assert(root?.startsWith('/root/.chariox/dev/'),'isolated lane state required');
assert(binary&&label&&evidence,'kernel binary, label, and evidence directory required');
mkdirSync(evidence,{recursive:true});
for(const sub of ['home','tmp','chariox','logs','history','work','config','cache','state'])mkdirSync(join(root,sub),{recursive:true,mode:0o700});
const port=async()=>{const s=createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const n=s.address().port;await new Promise(r=>s.close(r));return n};
const ports=await Promise.all(Array.from({length:5},port));
writeFileSync(join(root,'chariox/config.toml'),`[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[workflow]\nsession_default_max_agents = 64\n[state]\npath = "${root}/chariox/state.db"\n`,{mode:0o600});
const env={PATH:process.env.PATH,HOME:join(root,'home'),TMPDIR:join(root,'tmp'),IS_SANDBOX:'1',SHELL:'/bin/bash',CHARIOX_PROVIDER_DEV_STUB:'1',CHARIOX_HOME:join(root,'chariox'),CHARIOX_LOG_DIR:join(root,'logs'),CHARIOX_SESSION_HISTORY_DIR:join(root,'history'),CHARIOX_DAEMON_SOCKET:join('/var/tmp/jobsview',label+'.sock'),CHARIOX_DAEMON_ALIAS:'jobsview-'+label,CHARIOX_MACHINE_ALIAS:'jobsview-'+label,CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_OPENCODE_PORT:String(ports[2]),CHARIOX_CODEX_PORT:String(ports[3]),CHARIOX_KERNEL_BROWSER_PORT:String(ports[4]),XDG_CONFIG_HOME:join(root,'config'),XDG_CACHE_HOME:join(root,'cache'),XDG_STATE_HOME:join(root,'state')};
let child,client;const out={label,mode,at:new Date().toISOString(),cells:{},binary_sha256:createHash('sha256').update(readFileSync(binary)).digest('hex'),head:flags.get('--head')};
const save=()=>writeFileSync(join(evidence,'measurements.json'),JSON.stringify(out,null,2)+'\n');
const log=openSync(join(evidence,'kernel.log'),'a',0o600);
async function bounded(p,label,ms=120000){let t;try{return await Promise.race([p,new Promise((_,reject)=>t=setTimeout(()=>reject(Error(label+' deadline')),ms))])}finally{clearTimeout(t)}}
async function start(){child=spawn(binary,[],{env,stdio:['ignore',log,log]});client=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:env,controlRequestRetryDeadlineMs:1000});for(let i=0;i<120;i++){assert(child.exitCode===null&&child.signalCode===null,'kernel exited');try{if((await client.send({RelayStatus:null})).RelayStatus)return}catch{}await delay(250)}throw Error('startup deadline')}
async function stop(){await client?.close();if(child?.exitCode===null&&child.signalCode===null){assert(Number.isSafeInteger(child.pid)&&child.pid>1);child.kill('SIGTERM');for(let i=0;i<100&&child.exitCode===null&&child.signalCode===null;i++)await delay(100);if(child.exitCode===null&&child.signalCode===null)child.kill('SIGKILL');await bounded(new Promise(r=>child.exitCode!==null||child.signalCode!==null?r():child.once('exit',r)),'stop',10000)}}
const rpc=async request=>{const value=await bounded(client.send(request),Object.keys(request)[0]);if(value.Error)throw Error(value.Error.message);return value};
async function until(check, why, ms=30000) {
 const end=Date.now()+ms;
 while(Date.now()<end){assert(child.exitCode===null&&child.signalCode===null,'kernel exited while '+why);if(await check())return;await delay(100)}
 throw Error(why+' deadline');
}
async function caller(room,agent,attachment,run){
 const fx=bindProviderMcpFixture(run,{client,requests,sessionId:room,attachmentId:attachment});
 await fx.fixtureMcp.call('fixture/input-digest',{}, {timeoutMs:10000});
 let origin,taskId;
 return async(name,args)=>{
  for(let attempt=0;attempt<4;attempt++){
   const state=(await rpc({GetSessionState:{session_id:room}})).SessionState;
   const task=(state.agent_tasks??state.session.agent_tasks??[]).find(t=>t.agent_id===agent&&t.state==='working');assert(task,'active fixture task');
   if(taskId!==task.task_id){taskId=task.task_id;origin=task.prompt_id}
   let settled=false;
   const pending=fx.fixtureMcp.call('tools/call',{name,arguments:{...args,task_id:taskId,origin_prompt_id:origin}},{timeoutMs:120000,rawEnvelope:true}).finally(()=>{settled=true});
   const consent=(async()=>{while(!settled){
    const current=Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0];assert((current.provider_run??current).state==='Running','provider fixture exited during MCP request');
    const state=(await rpc({GetSessionState:{session_id:room}})).SessionState;
    for(const interaction of state.session.active_interactions??[]){
     if(interaction.agent_id===agent&&interaction.id.startsWith('watched-process-'))await rpc({RespondToInteraction:{session_id:room,interaction_id:interaction.id,choice_id:'allow_agent'}});
    }
    if(!settled)await delay(100);
   }})();
   let response;
   try{response=await Promise.race([pending,consent.then(()=>pending)])}finally{settled=true;await consent}
   if(response.error){const next=response.error.message?.match(/origin_prompt_id is `([^`]+)`/);if(next){origin=next[1];continue}throw Error(response.error.message)}
   const value=response.result??response;assert(!value.isError,JSON.stringify(value.content).slice(0,300));
   return value.structuredContent??JSON.parse(value.content.find(c=>c.type==='text').text);
  }
  throw Error('fixture turn changed repeatedly');
 };
}
try{
 await start();
 const made=(await rpc({CreateSession:{workspace_id:join(root,'work'),worktree_id:join(root,'work'),alias:'admission',agent_defaults:{provider:'dev-stub',account_profile:'default',model:'runtime-mcp-fixture',permission_level:'yolo'}}})).SessionCreated;
 const room=made.session.id, agents=[made.agent.id];
 for(let i=1;i<11;i++){
  const made=await rpc(requests.spawnAgentRequest(room,'dev-stub','load-'+i,'runtime-mcp-fixture',undefined,'low','build','yolo',undefined,undefined,undefined,'default'));
  assert(made.AgentSpawned?.agent?.id,'agent spawned');agents.push(made.AgentSpawned.agent.id);
 }
 const tools=[];
 for(const agent of agents){
  const attachment=(await rpc({AttachToSession:{session_id:room,client_id:'owner-'+agent,capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
  const launch=await rpc(requests.launchProviderRunsRequest([{sessionId:room,provider:'dev-stub',accountProfile:'default',model:'runtime-mcp-fixture',effort:'low',agentId:agent,native:{nativeTui:false}}],1));
  let run=launch.ProviderRunsLaunchAccepted.provider_runs[0].provider_run;
  await until(async()=>{const r=Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0];const next=r.provider_run??r;if(next.state==='Running'){run=next;return true}assert(!['Stopped','Failed','Exited'].includes(next.state),'provider fixture ended during launch');return false},'provider fixture running');
  await rpc(requests.submitPromptRequest(room,attachment,agent,'go',[]));
  await until(async()=>{const s=(await rpc({GetSessionState:{session_id:room}})).SessionState;return(s.agent_tasks??s.session.agent_tasks??[]).some(t=>t.agent_id===agent&&t.state==='working')},'fixture task working');
  tools.push(await caller(room,agent,attachment,run));
 }
 const terminal=async(operation)=>{const r=await rpc({RunCommand: { target: { kind: 'room_workspace', session_id: room }, operation}});return r.RunCommand.result};
 const list=async()=>(await terminal({operation:'exec_list'})).execs;
 // A never-read pipe must expire without any list/status operation renewing its lease.
 const abandoned=(await terminal({operation:'exec_open',command:['/bin/sleep','3600']})).exec_id;
 const abandonedAt=performance.now();
 // Finished output is intentionally uncollected. It must not consume one slot per command.
 const retained=[];
 for(let i=0;i<40;i++)retained.push((await terminal({operation:'exec_open',command:['/usr/bin/head','-c','524288','/dev/zero']})).exec_id);
 await until(async()=>{const rows=await list();return retained.every(id=>rows.some(r=>r.exec_id===id&&r.state.kind==='finished'))},'forty unread commands finished',60000);
 const buffered=(await list()).filter(r=>retained.includes(r.exec_id)).reduce((n,r)=>n+r.buffered_bytes,0);
 assert(buffered<=16*1024*1024,'kernel-wide unread output byte ceiling');
 let lossMarked=false;let lossGuidance=false;
 for(const id of retained){
  const page=(await terminal({operation:'exec_read',exec_id:id,after_cursor:0,wait_ms:0})).output;
  lossMarked ||= page.chunks.some(c=>Buffer.from(c.data_base64,'base64').toString().includes('Older unread command output was dropped'));
  lossGuidance ||= page.chunks.some(c=>/shared output buffer filled.*Read output promptly.*lease expires/.test(Buffer.from(c.data_base64,'base64').toString()));
 }
 assert(lossMarked,'unread loss marker');assert(lossGuidance,'byte pressure and collection/lease next step');
 const extra=(await terminal({operation:'exec_open',command:['/bin/true']})).exec_id;
 out.cells.C3={passed:true,finished:40,buffered_bytes:buffered,ceiling_bytes:16*1024*1024,loss_marked:lossMarked,new_exec:extra};save();
 // Eleven fixture agents use the real runtime MCP admission path, alongside 41 retained pipes.
 const launched=[];
 for(let i=0;i<30;i++)launched.push(await tools[i%11]('chariox.jobs.run',{command:['/bin/sleep','60'],wait_s:0,timeout_s:90}));
 const ids=launched.map(j=>j.job_id);assert(ids.every(Boolean));
 await until(async()=>{const s=(await rpc({GetSessionState:{session_id:room}})).SessionState;return ids.every(id=>s.jobs.some(j=>j.job_id===id&&j.state==='running'))},'all thirty jobs start',30000);
 out.cells.C1={passed:true,agents:11,jobs:30,job_ids:ids,ordinary_retained:41,all_running:true};save();
 for(let i=0;i<11;i++){
  const own=await tools[i]('chariox.jobs.list',{});
  assert(own.jobs.length>0&&own.jobs.every(job=>job.agent_id===agents[i]),'own list excludes peers without sudo');
  assert(ids.filter((_,j)=>j%11===i).every(id=>own.jobs.some(job=>job.job_id===id)),'every own job is discoverable');
 }
 for(let i=0;i<ids.length;i++)await tools[i%11]('chariox.jobs.cancel',{job_id:ids[i]});
 await until(async()=>{const s=(await rpc({GetSessionState:{session_id:room}})).SessionState;return ids.every(id=>s.jobs.some(j=>j.job_id===id&&j.result))},'load jobs settled');
 // Discover the running kernel's actual admission headroom, including its delegated scope.
 const busyJobs=[];let firstQueued;
 for(let i=0;i<128;i++){
  const job=await tools[0]('chariox.jobs.run',{command:['/bin/sleep','600'],needs:{memory_gib:1},wait_s:0,timeout_s:660});
  let state;
  await until(async()=>{const s=(await rpc({GetSessionState:{session_id:room}})).SessionState;state=s.jobs.find(j=>j.job_id===job.job_id);return state&&(state.result||state.state==='running'||state.state==='queued')},'memory reservation admitted or queued');
  assert(!state.result,'a fitting job must queue rather than be refused');
  if(state.state==='queued'){firstQueued=job.job_id;break}
  busyJobs.push(job.job_id);
 }
 assert(firstQueued&&busyJobs.length,'memory reservations eventually queue without a command count refusal');
 await tools[0]('chariox.jobs.cancel',{job_id:firstQueued});
 const waiting=await tools[1]('chariox.jobs.run',{command:['/bin/echo','memory-freed'],needs:{memory_gib:1},wait_s:0,timeout_s:30});
 let queue;
 await until(async()=>{const rows=await list();queue=rows.find(r=>r.command_line.includes('memory-freed')&&r.state.kind==='queued');return !!queue},'memory pressure queues next job');
 assert.equal(queue.state.wait.reason,'memory');
 assert(queue.state.wait.need_bytes>queue.state.wait.free_bytes,'explicit memory admission reason');
 const ownWaiting=await tools[1]('chariox.jobs.list',{});
 const ownQueue=ownWaiting.waiting.find(job=>job.job_id===waiting.job_id);
 assert(ownQueue&&ownQueue.pressure.reason==='memory'&&ownQueue.pressure.need_bytes>ownQueue.pressure.free_bytes,'own queued pressure');
 assert.match(ownQueue.next_action,/Wait.*chariox.jobs.cancel/);
 assert(ownWaiting.jobs.every(job=>job.agent_id===agents[1]),'pressure view excludes peers');
 const statusWaiting=await tools[1]('chariox.jobs.status',{job_id:waiting.job_id});
 assert.equal(statusWaiting.waiting.reason,'memory');assert.match(statusWaiting.next_action,/Wait/);
 out.cells.C5={passed:true,own_list_agents:11,no_sudo:true,pressure:ownQueue.pressure,next_action:ownQueue.next_action,output_loss_guidance:lossGuidance};save();
 await tools[0]('chariox.jobs.cancel',{job_id:busyJobs[0]});
 let completed;
 await until(async()=>{completed=await tools[1]('chariox.jobs.status',{job_id:waiting.job_id});return !!completed.result},'queued job starts after memory frees');
 assert.equal(completed.result.exit_code,0);assert.match(completed.result.output_tail,/memory-freed/);
 out.cells.C2={passed:true,reservations:busyJobs.length,memory_gib_each:1,queue:queue.state,started_after_release:true,job_id:waiting.job_id};save();
 for(const id of busyJobs.slice(1))await tools[0]('chariox.jobs.cancel',{job_id:id});
 for(const id of [...retained,extra])await terminal({operation:'exec_close',exec_id:id});
 await until(async()=>{await delay(1000);return !(await list()).some(r=>r.exec_id===abandoned)},'abandoned reader lease',360000);
 const elapsed=performance.now()-abandonedAt;
 assert(elapsed>=299000&&elapsed<360000,'ordinary reader expires after five minutes');
 out.cells.C4={passed:true,exec_id:abandoned,elapsed_ms:elapsed,record_absent:true};
 out.passed=true;save();
} catch(error){out.error=String(error.message);save();throw error}
finally{await stop();rmSync(root,{recursive:true,force:true})}
