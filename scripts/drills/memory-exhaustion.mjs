// Memory exhaustion drill (row 520): a kernel in an 8 GB systemd scope; real runtime jobs; the runaway is throttled, paused and stopped while the kernel and the other agents run on.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdirSync,writeFileSync,openSync,rmSync,readFileSync,readdirSync,cpSync} from 'node:fs';
import {createServer} from 'node:net';

import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const tree=fileURLToPath(new URL('../../',import.meta.url)).replace(/\/$/,'');
const {LocalIpcClient}=await import(tree+'/packages/kernel-client/dist/ipc.js');
const requests=await import(tree+'/packages/kernel-client/dist/ipc-requests.js');
const {bindProviderMcpFixture}=await import(tree+'/apps/cli/scripts/lib/provider-mcp-fixture.mjs');
const flags=new Map();for(let i=2;i<process.argv.length;i+=2)flags.set(process.argv[i],process.argv[i+1]);
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
// ---- Safety: refuse to run outside a systemd scope whose memory.max is at most 9 GiB (the emulated 8 GB machine).
const ownCgroup=readFileSync('/proc/self/cgroup','utf8').trim().split('\n').map(l=>l.replace(/^0::/,'')).find(l=>l.startsWith('/'));
const limits=[];for(let dir='/sys/fs/cgroup'+ownCgroup;dir.startsWith('/sys/fs/cgroup')&&dir!=='/sys/fs/cgroup';dir=join(dir,'..')){try{const v=Number(readFileSync(join(dir,'memory.max'),'utf8').trim());if(Number.isFinite(v))limits.push(v)}catch{}}
assert(limits.length&&Math.min(...limits)<=9*2**30,'run inside `systemd-run -p MemoryMax=8G -p Delegate=yes -p DelegateSubgroup=supervisor` (outer safety net)');
// What deploy/local-linux/start-kernel.sh does for the shipped unit: the unit hands its controllers down before the kernel starts.
writeFileSync('/sys/fs/cgroup'+ownCgroup.replace(/\/supervisor$/,'')+'/cgroup.subtree_control','+cpu +memory +pids');
const GiB=2**30, MiB=2**20, tag=flags.get('--tag')??'drill';
const budgetHigh=Number(flags.get('--budget-high-gib')??6)*GiB; // emulates a machine whose workloads thrash near full
const buildGiB=Number(flags.get('--build-gib')??6.8);
const nodeAlloc=(rateMiB,limitMiB,holdMs,extra='')=>`const a=[];let n=0;const t=setInterval(()=>{for(let i=0;i<${Math.round(rateMiB/20)}&&n<${limitMiB};i++,n++)a.push(Buffer.alloc(1<<20,1));if(n>=${limitMiB}){clearInterval(t);setTimeout(()=>process.exit(0),${holdMs})}},50);${extra}`;
const bystander=file=>`const b=Buffer.alloc(256<<20,1);const fs=require('fs');setInterval(()=>fs.appendFileSync(${JSON.stringify(file)},Date.now()+'\\n'),200);void b`;
const kernelCgroup=()=>{const l=readFileSync(`/proc/${child.pid}/cgroup`,'utf8').trim().replace(/^0::/,'');return '/sys/fs/cgroup'+l};
const budgets=()=>{const parent=join(kernelCgroup(),'..');return readdirSync(parent).filter(n=>n.startsWith(`chariox-${child.pid}-`)).map(n=>join(parent,n))};
// The emulated machine thrashes from here: the whole scope (kernel included) is throttled above this line, as a host out of memory is.
const emulateThrash=()=>{const b=join(kernelCgroup(),'..');try{writeFileSync(join(b,'memory.high'),String(budgetHigh));out.thrash={applied:b}}catch(e){out.thrash_error=String(e.message)}};
const relayPulse=async(stopSignal,out)=>{while(!stopSignal.done){const t=performance.now();try{await bounded(client.send({RelayStatus:null}),'relay',5000);out.max=Math.max(out.max??0,performance.now()-t);out.ok=(out.ok??0)+1}catch(e){out.failed=(out.failed??0)+1}await delay(500)}};
const gaps=file=>{const t=readFileSync(file,'utf8').trim().split('\n').map(Number);return t.slice(1).reduce((m,x,i)=>Math.max(m,x-t[i]),0)};
const kernelLog=()=>{let t=readFileSync(join(evidence,'kernel.log'),'utf8');try{for(const f of readdirSync(join(root,'logs')))t+='\n'+readFileSync(join(root,'logs',f),'utf8')}catch{}return t};
let thrash,beat;
try{
 await start();
 // M4: the kernel's own memory is protected (memory.min of the scope above its supervisor subgroup).
 // The kernel's own (supervisor) cgroup and the unit above it must both carry the minimum: the agents' budgets are its siblings.
 const readMin=()=>{const rd=f=>{try{return Number(readFileSync(f,'utf8'))}catch{return 0}};const own=join(kernelCgroup(),'memory.min'),unit=join(kernelCgroup(),'..','memory.min');return{file:own,unit_file:unit,own_bytes:rd(own),unit_bytes:rd(unit),bytes:Math.min(rd(own),rd(unit))}};
 for(let i=0;i<40&&readMin().bytes<2*GiB;i++)await delay(500); // a kernel without a set MemoryMin raises its own at start
 for(let i=0;i<20&&!/kernel's memory is protected/.test(kernelLog());i++)await delay(500);
 const protectedNow=readMin();
 out.cells.M4={memory_min_bytes:protectedNow.bytes,kernel_cgroup_memory_min:protectedNow.own_bytes,unit_memory_min:protectedNow.unit_bytes,file:protectedNow.file,scope_started_with_memory_min:process.env.KERNEL_MEMORY_MIN??null,kernel_log_says_protected:/kernel's memory is protected/.test(kernelLog()),raised_by_kernel:/raised by the kernel itself/.test(kernelLog())};
 if(process.env.STRICT_M4!=='0'){assert(out.cells.M4.memory_min_bytes>=2*GiB,'kernel memory.min >= 2 GiB');assert(out.cells.M4.kernel_log_says_protected,'kernel says its memory is protected')}save();
 const made=(await rpc({CreateSession:{workspace_id:join(root,'work'),worktree_id:join(root,'work'),alias:'exhaustion',agent_defaults:{provider:'dev-stub',account_profile:'default',model:'runtime-mcp-fixture',permission_level:'yolo'}}})).SessionCreated;
 const room=made.session.id, agents=[made.agent.id];
 for(let i=1;i<3;i++){const m=await rpc(requests.spawnAgentRequest(room,'dev-stub','exhaust-'+i,'runtime-mcp-fixture',undefined,'low','build','yolo',undefined,undefined,undefined,'default'));assert(m.AgentSpawned?.agent?.id,'agent spawned');agents.push(m.AgentSpawned.agent.id)}
 const tools=[],held=[];
 for(const agent of agents){
  const attachment=(await rpc({AttachToSession:{session_id:room,client_id:'owner-'+agent,capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
  held.push({session_id:room,attachment_id:attachment});
  const launch=await rpc(requests.launchProviderRunsRequest([{sessionId:room,provider:'dev-stub',accountProfile:'default',model:'runtime-mcp-fixture',effort:'low',agentId:agent,native:{nativeTui:false}}],1));
  let run=launch.ProviderRunsLaunchAccepted.provider_runs[0].provider_run;
  await until(async()=>{const r=Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0];const next=r.provider_run??r;if(next.state==='Running'){run=next;return true}return false},'provider fixture running');
  await rpc(requests.submitPromptRequest(room,attachment,agent,'go',[]));
  await until(async()=>{const s=(await rpc({GetSessionState:{session_id:room}})).SessionState;return(s.agent_tasks??s.session.agent_tasks??[]).some(t=>t.agent_id===agent&&t.state==='working')},'fixture task working');
  tools.push(await caller(room,agent,attachment,run));
 }
 // A terminal client keeps its attachments alive, as the real clients do (the kernel drops one silent for 30 s).
 beat=setInterval(()=>{rpc({HeartbeatAttachments:{attachments:held}}).catch(()=>{})},3000);
 const [runaway,steady1,steady2]=tools;
 const terminal=async(operation)=>{const r=await rpc({RoomTerminal:{session_id:room,operation}});return r.RoomTerminal.result};
 const finish=async(tool,job,why,ms=240000)=>{let st;await until(async()=>{st=await tool('chariox.jobs.status',{job_id:job});return !!st.result},why,ms);return st};
 const cell=process.env.CELLS??'M1,M2,M3,M6';
 // M1: a build using ~90% of the 8 GB machine, with no pressure behind it, completes untouched.
 if(cell.includes('M1')){
  const job=await steady1('chariox.jobs.run',{command:['node','-e',nodeAlloc(400,Math.round(buildGiB*1024),15000)],wait_s:0,timeout_s:240});
  const st=await finish(steady1,job.job_id,'the build-sized job ends',260000);
  out.cells.M1={build_gib:buildGiB,exit_code:st.result.exit_code,told:/chariox:/.test(JSON.stringify(st.result)),exhaustion_logged:/"component":"process_resources\.exhaustion"/.test(kernelLog())};
  assert.equal(st.result.exit_code,0,'build-sized job exits 0');assert(!out.cells.M1.told,'it was told nothing');assert(!out.cells.M1.exhaustion_logged,'nothing was acted on');save();
 }
 // M2: a runaway among two steady agents is acted on; only it; the kernel stays connected.
 // From here the workloads' cgroup is limited below the machine, as a thrashing machine behaves (M1 ran without it).
 // The budget cgroup appears with the first workload, and a new one after a hot reload: apply it as it appears.
 thrash=cell.includes('M2')||cell.includes('M3')?setInterval(emulateThrash,200):null;
 const runawayCell=async(label,start,done)=>{
  const file1=join(root,`hb-${label}-1`),file2=join(root,`hb-${label}-2`);
  const a=await steady1('chariox.jobs.run',{command:['node','-e',bystander(file1)],wait_s:0,timeout_s:400});
  const b=await steady2('chariox.jobs.run',{command:['node','-e',bystander(file2)],wait_s:0,timeout_s:400});
  await delay(3000);
  const pulse={},sig={done:false};const pulsing=relayPulse(sig,pulse);
  const target=await start();
  const result=await done(target);
  await delay(2000);sig.done=true;await pulsing;
  const survivors=[];for(const [tool,job] of [[steady1,a.job_id],[steady2,b.job_id]]){const st=await tool('chariox.jobs.status',{job_id:job});survivors.push(!st.result)}
  out.cells[label]={...result,relay:pulse,bystander_gap_ms:[gaps(file1),gaps(file2)],bystanders_running:survivors};
  assert(!pulse.failed,'the kernel stayed connected');assert(pulse.max<5000,'the kernel answered within 5 s');assert(survivors.every(Boolean),'both bystanders still run');
  for(const [tool,job] of [[steady1,a.job_id],[steady2,b.job_id]])await tool('chariox.jobs.cancel',{job_id:job});
  save();
 };
 if(cell.includes('M2')){
  await runawayCell('M2',()=>runaway('chariox.jobs.run',{command:['node','-e',nodeAlloc(300,64*1024,0)],wait_s:0,timeout_s:300}),async job=>{
   const st=await finish(runaway,job.job_id,'the runaway is stopped');
   const text=JSON.stringify(st.result);
   let inbox,notices;
   for(let i=0;i<30;i++){ // the agent hears the stop a moment after the job ends
    inbox=await runaway('chariox.events.inbox',{limit:64});
    notices=(inbox.events??[]).filter(e=>e.kind==='resource_notice').map(e=>e.payload?.message);
    if(notices.some(m=>/was stopped/.test(m)))break;await delay(1000);
   }
   out.inbox_raw=JSON.stringify(inbox).slice(0,2500);save();
   const log=kernelLog().split('\n').filter(l=>l.includes('"component":"process_resources.exhaustion"'));
   const r={exit:st.result.exit_code,result_has_reason:/was stopped|ran out of memory/.test(text),agent_notices:notices,log_steps:log.length};
   out.cells.M2_partial={...r,result:text.slice(-700)};save();
   assert(r.result_has_reason,'the stopped job result carries the reason');
   assert(notices.some(m=>/was throttled/.test(m))&&notices.some(m=>/was stopped/.test(m)),'agent heard throttle and stop');
   assert(notices.every(m=>/grew|out of memory|ran out/.test(m)),'notices carry the numbers');
   return r;
  });
 }
 // M3: the same after a hot reload (SIGUSR2): same pid, protection still holds.
 if(cell.includes('M3')){
  const pid=child.pid;process.kill(pid,'SIGUSR2');
  await until(async()=>{try{return !!(await client.send({RelayStatus:null})).RelayStatus}catch{return false}},'kernel answers after the reload',120000);
  assert.equal(child.pid,pid);assert(child.exitCode===null,'kernel still running');await delay(3000);
  const runExec=async()=>(await terminal({operation:'exec_open',command:['node','-e',nodeAlloc(300,64*1024,0)]})).exec_id;
  let id;const pulse={},sig={done:false};const pulsing=relayPulse(sig,pulse);
  id=await runExec();
  await until(async()=>{const row=(await terminal({operation:'exec_list'})).execs.find(r=>r.exec_id===id);return row&&row.state.kind==='finished'},'the runaway Room command is stopped after the reload',240000);
  sig.done=true;await pulsing;
  const page=(await terminal({operation:'exec_read',exec_id:id,after_cursor:0,wait_ms:0})).output;
  const text=page.chunks.map(c=>Buffer.from(c.data_base64,'base64').toString()).join('');
  out.cells.M3={same_pid:true,relay:pulse,told:/was stopped|ran out of memory/.test(text),excerpt:text.slice(-400)};
  assert(out.cells.M3.told,'stopped after the reload with the reason');assert(!pulse.failed,'kernel stayed connected');save();
 }
 out.passed=true;save();
} catch(error){out.error=String(error.message);save();throw error}
finally{clearInterval(thrash);clearInterval(beat);await stop();try{cpSync(join(root,'logs'),join(evidence,'logs'),{recursive:true})}catch{}rmSync(root,{recursive:true,force:true})}
