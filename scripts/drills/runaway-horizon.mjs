// Runaway-horizon drill (row 541 round 2): a kernel in a 24 GB systemd unit; an agent command that takes memory at 1.3-2.7 GB/s is throttled, paused and stopped before the unit runs out; other work is untouched.
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
import {mkdirSync,writeFileSync,openSync,rmSync,readFileSync,readdirSync,cpSync,existsSync} from 'node:fs';
import {createServer} from 'node:net';

import {createHash,randomBytes} from 'node:crypto';
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
 return async(name,args,bare)=>{
  for(let attempt=0;attempt<4;attempt++){
   const state=(await rpc({GetSessionState:{session_id:room}})).SessionState;
   const task=(state.agent_tasks??state.session.agent_tasks??[]).find(t=>t.agent_id===agent&&t.state==='working');assert(task,'active fixture task');
   if(taskId!==task.task_id){taskId=task.task_id;origin=task.prompt_id}
   let settled=false;
   const pending=fx.fixtureMcp.call('tools/call',{name,arguments:bare?args:{...args,task_id:taskId,origin_prompt_id:origin}},{timeoutMs:120000,rawEnvelope:true}).finally(()=>{settled=true});
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
// ---- Safety: refuse to run outside a systemd unit whose memory.max is at most 20 GiB (the emulated machine).
const ownCgroup=readFileSync('/proc/self/cgroup','utf8').trim().split('\n').map(l=>l.replace(/^0::/,'')).find(l=>l.startsWith('/'));
const limits=[];for(let dir='/sys/fs/cgroup'+ownCgroup;dir.startsWith('/sys/fs/cgroup')&&dir!=='/sys/fs/cgroup';dir=join(dir,'..')){try{const v=Number(readFileSync(join(dir,'memory.max'),'utf8').trim());if(Number.isFinite(v))limits.push(v)}catch{}}
assert(limits.length&&Math.min(...limits)<=24*2**30,'run inside run-runaway-horizon.sh (a systemd unit with MemoryMax<=24G)');
writeFileSync('/sys/fs/cgroup'+ownCgroup.replace(/\/supervisor$/,'')+'/cgroup.subtree_control','+cpu +memory +pids');
const GiB=2**30, MiB=2**20;
const budgetHigh=Number(flags.get('--budget-high-gib')??12)*GiB; // workloads thrash above this line: a machine near its limit
const heavyGiB=Number(flags.get('--heavy-gib')??6), hogGiB=Number(flags.get('--hog-gib')??11);
// A command that allocates `gib` GiB quickly, holds it, and exits. `name` keeps the three paths' keys apart.
const alloc=(name,gib,holdMs,doneFile)=>`/*${name}*/const a=[];let n=0;const t=setInterval(()=>{for(let i=0;i<10&&n<${gib*1024};i++,n++)a.push(Buffer.alloc(1<<20,1));if(n>=${gib*1024}){clearInterval(t);setTimeout(()=>{${doneFile?`require('fs').writeFileSync('${doneFile}','0');`:''}process.exit(0)},${holdMs})}},10)`;
const runaway=(name)=>`/*${name}*/const a=[];let n=0;setInterval(()=>{for(let i=0;i<15;i++,n++)a.push(Buffer.alloc(1<<20,1))},50)`;
const kernelCgroup=()=>{const l=readFileSync(`/proc/${child.pid}/cgroup`,'utf8').trim().replace(/^0::/,'');return '/sys/fs/cgroup'+l};
const emulateThrash=()=>{const b=join(kernelCgroup(),'..');try{writeFileSync(join(b,'memory.high'),String(budgetHigh))}catch{}};
const relayPulse=async(sig,o)=>{while(!sig.done){const t=performance.now();try{await bounded(client.send({RelayStatus:null}),'relay',8000);o.max=Math.max(o.max??0,performance.now()-t);o.ok=(o.ok??0)+1}catch(e){o.failed=(o.failed??0)+1}await delay(250)}};
const records=()=>{const rows=[];try{for(const f of readdirSync(join(root,'logs')))for(const l of readFileSync(join(root,'logs',f),'utf8').split('\n')){if(!l)continue;try{rows.push(JSON.parse(l))}catch{}}}catch{}return rows};
const admissionLog=(re)=>records().filter(r=>r.component==='process_resources.admission'&&re.test(JSON.stringify(r.command??'')+' '+(r.message??'')));
const exhaustionLog=()=>records().filter(r=>r.component==='process_resources.exhaustion');
const peaksFile=()=>{const find=d=>{for(const e of readdirSync(d,{withFileTypes:true})){const p=join(d,e.name);if(e.isDirectory()){const r=find(p);if(r)return r}else if(e.name==='admission-peaks.json')return p}};return find(root)};
const peakOf=(mark)=>{const f=peaksFile();if(!f)return null;const all=JSON.parse(readFileSync(f,'utf8'));return Object.values(all).map(p=>p.memory).sort((a,b)=>b-a)};
const sh=(cmd)=>JSON.stringify(cmd);
let thrash,beat;
try{
 await start();
 const made=(await rpc({CreateSession:{workspace_id:join(root,'work'),worktree_id:join(root,'work'),alias:'admission',agent_defaults:{provider:'dev-stub',account_profile:'default',model:'runtime-mcp-fixture',permission_level:'yolo'}}})).SessionCreated;
 const room=made.session.id, agentId=made.agent.id;
 const attachment=(await rpc({AttachToSession:{session_id:room,client_id:'owner-'+agentId,capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
 const held=[{session_id:room,attachment_id:attachment}];
 const launch=await rpc(requests.launchProviderRunsRequest([{sessionId:room,provider:'dev-stub',accountProfile:'default',model:'runtime-mcp-fixture',effort:'low',agentId,native:{nativeTui:false}}],1));
 let run=launch.ProviderRunsLaunchAccepted.provider_runs[0].provider_run;
 await until(async()=>{const r=Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0];const next=r.provider_run??r;if(next.state==='Running'){run=next;return true}return false},'provider fixture running');
 await rpc(requests.submitPromptRequest(room,attachment,agentId,'go',[]));
 await until(async()=>{const s=(await rpc({GetSessionState:{session_id:room}})).SessionState;return(s.agent_tasks??s.session.agent_tasks??[]).some(t=>t.agent_id===agentId&&t.state==='working')},'fixture task working');
 const tool=await caller(room,agentId,attachment,run);
 beat=setInterval(()=>{rpc({HeartbeatAttachments:{attachments:held}}).catch(()=>{})},3000);
 const terminal=async(operation)=>{const r=await rpc({RoomTerminal:{session_id:room,operation}});return r.RoomTerminal.result};
 // The three launch shapes: an agent's Room command, a job, a typed terminal command.
 const execStart=async code=>(await terminal({operation:'exec_open',command:['node','-e',code]})).exec_id;
 const execRow=async id=>(await terminal({operation:'exec_list'})).execs.find(r=>r.exec_id===id);
 const execState=row=>row?.state?.kind??JSON.stringify(row?.state);
 const execDone=async(id,ms=240000)=>{let row;await until(async()=>{row=await execRow(id);return execState(row)==='finished'},'exec '+id+' finished',ms);return row};
 const execExit=async id=>{const page=(await terminal({operation:'exec_read',exec_id:id,after_cursor:0,wait_ms:0})).output;return page.outcome};
 const exitedOk=o=>o?.kind==='exited'&&o.code===0;
 const decisions=(mark,kinds)=>admissionLog(new RegExp(mark)).filter(r=>kinds.includes(r.decision)).length;
 const jobStart=async code=>(await tool('chariox.jobs.run',{command:['node','-e',code],wait_s:0,timeout_s:400})).job_id;
 const jobDone=async(id,ms=240000)=>{let st;await until(async()=>{st=await tool('chariox.jobs.status',{job_id:id});return !!st.result},'job '+id+' finished',ms);return st};
 const termOpen=async()=>{const r=await tool('chariox.terminal_open',{title:'admission'},true);out.term_open_raw=JSON.stringify(r).slice(0,400);save();return r.terminal_id??r.terminal?.terminal_id??r.terminal?.id??r.id};
 const termRun=async(tid,code,tag)=>{await tool('chariox.terminal_send',{terminal_id:tid,text:`node -e ${sh(code)}; echo A6RC$?END${tag}`,enter:true},true)};
 const termFile=join(root,'done-term');
 const termDone=async(tid,tag,ms=240000)=>{await until(async()=>existsSync(termFile)&&readFileSync(termFile,'utf8')==='0','the typed command ran to its end and exited 0 ('+tag+')',ms);rmSync(termFile,{force:true})};

 const cells=new Set((process.env.CELLS??'R1,R2,R3,R4,R5').split(','));
 const unitDir=()=>{for(let d=kernelCgroup();d.startsWith('/sys/fs/cgroup')&&d!=='/sys/fs/cgroup';d=join(d,'..')){try{if(Number.isFinite(Number(readFileSync(join(d,'memory.max'),'utf8').trim())))return d}catch{}}return kernelCgroup()};
 const oomKills=()=>{try{return Number((readFileSync(join(unitDir(),'memory.events'),'utf8').match(/^oom_kill (\d+)/m)??[])[1]??0)}catch{return -1}};
 const hogCode=(name,mibPerTick)=>`/*${name}*/const a=[];setInterval(()=>{for(let i=0;i<${mibPerTick};i++)a.push(Buffer.alloc(1<<20,1))},10)`;
 const slowCode=(name,gib,mibPerTick)=>`/*${name}*/const a=[];let n=0;const t=setInterval(()=>{for(let i=0;i<${mibPerTick}&&n<${gib*1024};i++,n++)a.push(Buffer.alloc(1<<20,1));if(n>=${gib*1024}){clearInterval(t);setTimeout(()=>process.exit(0),1500)}},100)`;
 const outputOf=async id=>{const page=(await terminal({operation:'exec_read',exec_id:id,after_cursor:0,wait_ms:0})).output;return page.chunks.map(c=>Buffer.from(c.data_base64,'base64').toString()).join('')};
 const horizonLog=()=>exhaustionLog().filter(r=>r.trigger==='runaway_horizon');
 const sig={done:false},pulse={};const pulsing=relayPulse(sig,pulse);
 // one hog run: the steady neighbour and a light command run beside it; what the net did, how fast, and what the unit saw.
 const hogRun=async(cell,mibPerTick)=>{
  const logged=horizonLog().length, oom0=oomKills();
  const neighbour=await execStart(alloc('neighbour_'+cell,2,40000));await delay(4000);
  const t0=performance.now();
  const hog=await execStart(hogCode('hog_'+cell,mibPerTick));
  let peak=0;const row=await (async()=>{let r;await until(async()=>{r=await execRow(hog);try{peak=Math.max(peak,Number(readFileSync(join(unitDir(),'memory.current'),'utf8')))}catch{}return execState(r)==='finished'},'the runaway was ended',120000);return r})();
  const endedMs=Math.round(performance.now()-t0);
  const text=await outputOf(hog);const outcome=await execExit(hog);
  const steps=horizonLog().slice(logged);
  const lightId=await execStart('/*light*/process.exit(0)');const lt=performance.now();await execDone(lightId,30000);const lightMs=Math.round(performance.now()-lt);
  const neighbourState=execState(await execRow(neighbour));
  await execDone(neighbour,120000);const neighbourOk=exitedOk(await execExit(neighbour));
  return {cell,mib_per_tick:mibPerTick,rate_gb_s:+(mibPerTick*100/1024).toFixed(2),ended_ms:endedMs,outcome,steps:steps.map(r=>({step:r.message,pid:r.pid,headroom_gib:+(r.headroom_bytes/GiB).toFixed(1),rate_gb_s:+(r.runaway_bytes_per_second/GiB).toFixed(2),resident_gib:+(r.resident_bytes/GiB).toFixed(1)})),told:text.slice(-700),unit_peak_gib:+(peak/GiB).toFixed(1),unit_oom_kills:oomKills()-oom0,neighbour_state_while_hog_ended:neighbourState,neighbour_ok:neighbourOk,light_ms:lightMs};
 };
 const verify=(r)=>{
  const kinds=r.steps.map(s=>s.step);
  assert(kinds.some(m=>/throttled/.test(m)),r.cell+': the net did not throttle the runaway at its horizon: '+JSON.stringify(kinds));
  assert(!exitedOk(r.outcome),r.cell+': the runaway ran to a normal exit');
  assert(/stopped|paused/.test(r.told)&&/chariox: your process/.test(r.told),r.cell+': the agent was not told in plain words: '+r.told);
  assert(!/panic|errno|EAGAIN|SIGKILL|signal 9/i.test(r.told),r.cell+': the reason carries raw error text: '+r.told);
  assert(kinds.some(m=>/paused/.test(m))&&kinds.some(m=>/stopped/.test(m)),r.cell+': the ladder did not pause and then stop the runaway: '+JSON.stringify(kinds));
  assert.equal(r.unit_oom_kills,0,r.cell+': the OS OOM killer acted before the net');
  assert(r.unit_peak_gib<22,r.cell+': the unit reached its wall ('+r.unit_peak_gib+' GiB of 24) before the runaway was ended');
  assert(r.neighbour_ok&&r.neighbour_state_while_hog_ended==='running',r.cell+': another command was touched');
  assert(r.light_ms<5000,r.cell+': a light command waited');
 };
 if(cells.has('R1')){
  for(const [name,m] of [['R1_1.3GBs',13],['R1_2.7GBs',27]]){const r=await hogRun(name,m);out.cells[name]=r;save();verify(r)}
 }
 // R2: the kernel kept answering during the runaway (the relay pulse ran through R1).
 if(cells.has('R2')){
  out.cells.R2={relay:pulse};save();
  assert(!pulse.failed&&pulse.max<8000,'R2: the kernel stopped answering: '+JSON.stringify(pulse));
 }
 // R3 (D-GOOD): healthy heavy work is never touched: a 6 GiB command at ~100 MiB/s with room, and a 2 GiB burst at 1.3 GB/s with room.
 if(cells.has('R3')){
  const before=horizonLog().length+exhaustionLog().length;
  const slow=await execStart(slowCode('good_slow',6,10));
  const burst=await execStart(slowCode('good_burst',2,130));
  await execDone(burst,120000);await execDone(slow,240000);
  const okSlow=exitedOk(await execExit(slow)),okBurst=exitedOk(await execExit(burst));
  out.cells.R3={slow_6gib_100mibs_ok:okSlow,burst_2gib_1p3gbs_ok:okBurst,net_lines_during:horizonLog().length+exhaustionLog().length-before};save();
  assert(okSlow&&okBurst,'R3: a healthy command was stopped or failed');
  assert.equal(out.cells.R3.net_lines_during,0,'R3: the net acted on healthy work');
 }
 // R4: the same after a hot reload (SIGUSR2, same pid).
 if(cells.has('R4')){
  const pid=child.pid;process.kill(pid,'SIGUSR2');
  await until(async()=>{try{return !!(await client.send({RelayStatus:null})).RelayStatus}catch{return false}},'kernel answers after the reload',120000);
  assert.equal(child.pid,pid);await delay(3000);
  const r=await hogRun('R4_after_reload',13);out.cells.R4=r;save();verify(r);
 }
 // R5: one owner per decision: no pid is acted on by both admission and exhaustion.
 if(cells.has('R5')){
  const ex=new Set(exhaustionLog().map(r=>r.pid));
  const adm=admissionLog(/./).filter(r=>r.decision==='paused'||r.decision==='held');
  const both=adm.filter(r=>ex.has(r.pid)).map(r=>({pid:r.pid,decision:r.decision}));
  out.cells.R5={exhaustion_pids:[...ex],admission_paused_or_held:adm.length,double_action:both};save();
  assert.equal(both.length,0,'R5: a command was acted on by both policies');
 }
 sig.done=true;await pulsing;out.relay_all=pulse;
 out.exhaustion_lines=exhaustionLog().map(r=>r.message+' | '+r.trigger);
 out.passed=true;save();
} catch(error){out.error=String(error.message);save();throw error}
finally{clearInterval(thrash);clearInterval(beat);await stop();try{cpSync(join(root,'logs'),join(evidence,'logs'),{recursive:true})}catch{}rmSync(root,{recursive:true,force:true})}
