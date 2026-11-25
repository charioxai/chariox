// Admission drill (row 541): a kernel in a 16 GB systemd unit; commands that peaked heavy before wait before they grow, light ones never wait, a runaway is acted on, the kernel keeps answering. Cells A1-A6 of row 541.
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
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
assert(limits.length&&Math.min(...limits)<=20*2**30,'run inside run-admission-holds.sh (a systemd unit with MemoryMax<=20G)');
writeFileSync('/sys/fs/cgroup'+ownCgroup.replace(/\/supervisor$/,'')+'/cgroup.subtree_control','+cpu +memory +pids');
const GiB=2**30, MiB=2**20;
const budgetHigh=Number(flags.get('--budget-high-gib')??12)*GiB; // workloads thrash above this line: a machine near its limit
const heavyGiB=Number(flags.get('--heavy-gib')??6), hogGiB=Number(flags.get('--hog-gib')??11);
// A command that allocates `gib` GiB quickly, holds it, and exits. `name` keeps the three paths' keys apart.
const alloc=(name,gib,holdMs)=>`/*${name}*/const a=[];let n=0;const t=setInterval(()=>{for(let i=0;i<10&&n<${gib*1024};i++,n++)a.push(Buffer.alloc(1<<20,1));if(n>=${gib*1024}){clearInterval(t);setTimeout(()=>process.exit(0),${holdMs})}},10)`;
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
 const jobStart=async code=>(await tool('chariox.jobs.run',{command:['node','-e',code],wait_s:0,timeout_s:400})).job_id;
 const jobDone=async(id,ms=240000)=>{let st;await until(async()=>{st=await tool('chariox.jobs.status',{job_id:id});return !!st.result},'job '+id+' finished',ms);return st};
 const termOpen=async()=>{const r=await tool('chariox.terminal_open',{title:'admission'},true);out.term_open_raw=JSON.stringify(r).slice(0,400);save();return r.terminal_id??r.terminal?.terminal_id??r.terminal?.id??r.id};
 const termRun=async(tid,code,tag)=>{await tool('chariox.terminal_send',{terminal_id:tid,text:`node -e ${sh(code)}; echo ${tag.slice(0,2)}$((1+1))${tag.slice(2)}`,enter:true},true)};
 const termDone=async(tid,tag,ms=240000)=>{await tool('chariox.terminal_wait',{terminal_id:tid,until:[{text:`${tag.slice(0,2)}2${tag.slice(2)}`}],timeout_seconds:Math.floor(ms/1000)},true)};
 const cell=process.env.CELLS??'A6,A1,A3,A2,A4,A5';
 const peaks0=peakOf();out.peaks_before=peaks0;save();
 // A6: an unknown command grows and ends: its peak is recorded; its next run waits while a hog holds the room.
 const learn=async(path)=>{
  const start=path==='exec'?execStart:path==='job'?jobStart:null;
  const code=alloc('path_'+path,heavyGiB,1500);
  const before=(peakOf()??[]).filter(m=>m>=(heavyGiB-1)*GiB).length;
  if(path==='term'){const tid=await termOpen();await termRun(tid,code,'A6TERMDONE');await delay(3000);const ps=()=>{const pids=[];const walk=d=>{try{for(const p of readFileSync(join(d,'cgroup.procs'),'utf8').split('\n'))if(p)pids.push(p)}catch{}try{for(const e of readdirSync(d,{withFileTypes:true}))if(e.isDirectory())walk(join(d,e.name))}catch{}};walk(join(kernelCgroup(),'..'));return pids.length?execFileSync('ps',['-o','pid,ppid,pgid,stat,rss,comm','-p',pids.join(',')],{encoding:'utf8'}).slice(0,4000):''};out.ps_term_running=ps();await termDone(tid,'A6TERMDONE');await delay(2000);out.ps_term_after=ps();out.term_id=tid;save()}
  else{const id=await start(code);path==='exec'?await execDone(id):await jobDone(id)}
  let recorded=false;for(let i=0;i<40&&!recorded;i++){recorded=(peakOf()??[]).filter(m=>m>=(heavyGiB-1)*GiB).length>before;if(!recorded)await delay(250)}
  if(!recorded){await delay(5000);out['peaks_raw_'+path]=peaksFile()?readFileSync(peaksFile(),'utf8'):null;save()}
  return {path,recorded,peaks_gib:(peakOf()??[]).map(m=>+(m/GiB).toFixed(2)).slice(0,6)};
 };
 const heldNext=async(path,nth)=>{
  // A hog takes most of the pool; the next run of the learned command cannot fit and waits, then starts when the hog ends.
  const hog=await execStart(alloc('hog_'+path,hogGiB,25000));
  await delay(13000);
  const code=alloc('path_'+path,heavyGiB,1500);
  let id,tid,started=performance.now();
  const states=new Set();
  if(path==='term'){tid=await termOpen();await termRun(tid,code,'A6TERM'+nth+'DONE')}
  else id=path==='exec'?await execStart(code):await jobStart(code);
  await delay(5000);
  const heldLines=admissionLog(new RegExp('path_'+path)).filter(r=>r.decision==='held').length;
  if(path==='exec')states.add(execState(await execRow(id)));
  await execDone(hog,120000);
  if(path==='term')await termDone(tid,'A6TERM'+nth+'DONE');else path==='exec'?await execDone(id):await jobDone(id);
  const startedLines=admissionLog(new RegExp('path_'+path)).filter(r=>r.decision==='started'||r.decision==='resumed').length;
  return {path,held_lines_while_hog_ran:heldLines,started_lines_after:startedLines,exec_state_while_hog_ran:[...states],waited_ms:Math.round(performance.now()-started)};
 };
 if(cell.includes('A6')){
  out.cells.A6={learn:[],held:[]};
  for(const path of ['exec','job','term']){
   out.cells.A6.learn.push(await learn(path));save();
  }
  for(const [i,path] of ['exec','job','term'].entries()){
   if(!out.cells.A6.learn.find(l=>l.path===path)?.recorded)continue;
   out.cells.A6.held.push(await heldNext(path,i));save();
  }
 }
 // A1/A3: four parallel runs of a command that peaked at heavyGiB: two fit, the others wait and start in turn; a light command never waits.
 const sig={done:false},pulse={};const pulsing=relayPulse(sig,pulse);
 if(cell.includes('A1')){
  const peaksSeen=[];const ids=[];
  const code=alloc('path_exec',heavyGiB,4000);
  for(let i=0;i<4;i++)ids.push(await execStart(code));
  let light,lightMs;
  const t0=performance.now();let maxRunning=0,maxQueued=0;
  const lightId=await execStart('process.exit(0)'.replace(/^/,'/*light*/'));
  const lt=performance.now();light=await execDone(lightId,30000);lightMs=Math.round(performance.now()-lt);
  while(true){
   const rows=await Promise.all(ids.map(execRow));const states=rows.map(execState);
   maxRunning=Math.max(maxRunning,states.filter(s=>s==='running'||s==='starting').length);
   maxQueued=Math.max(maxQueued,states.filter(s=>s!=='running'&&s!=='starting'&&s!=='finished').length);
   if(states.every(s=>s==='finished'))break;
   assert(performance.now()-t0<300000,'A1 deadline');await delay(200);
  }
  const rows=await Promise.all(ids.map(execRow));
  out.cells.A1={runs:4,heavy_gib:heavyGiB,max_running:maxRunning,max_queued:maxQueued,light_ms:lightMs,light_state:execState(light),rows:rows.map(r=>({state:execState(r),outcome:r?.outcome??r?.state})),held_lines:admissionLog(/path_exec/).filter(r=>r.decision==='held').length,started_lines:admissionLog(/path_exec/).filter(r=>r.decision==='started').length};
  save();
 }
 // A2: a runaway started from an agent shell is acted on before the machine runs short; the kernel keeps answering.
 if(cell.includes('A2')){
  thrash=setInterval(emulateThrash,200);
  const p2={},sig2={done:false};const pulsing2=relayPulse(sig2,p2);
  const id=await execStart(runaway('runaway_a2'));
  const row=await execDone(id,240000);
  const page=(await terminal({operation:'exec_read',exec_id:id,after_cursor:0,wait_ms:0})).output;
  const text=page.chunks.map(c=>Buffer.from(c.data_base64,'base64').toString()).join('');
  sig2.done=true;await pulsing2;
  out.cells.A2={row:row?.outcome??row?.state,told:/stopped|paused|memory/.test(text),excerpt:text.slice(-500),relay:p2,exhaustion_steps:exhaustionLog().map(r=>r.message)};
  save();
 }
 // A4: the same after a hot reload (SIGUSR2, same pid).
 if(cell.includes('A4')){
  const pid=child.pid;process.kill(pid,'SIGUSR2');
  await until(async()=>{try{return !!(await client.send({RelayStatus:null})).RelayStatus}catch{return false}},'kernel answers after the reload',120000);
  assert.equal(child.pid,pid);await delay(3000);
  const code=alloc('path_exec',heavyGiB,4000);const ids=[];for(let i=0;i<4;i++)ids.push(await execStart(code));
  const lt=performance.now();const lightId=await execStart('/*light*/process.exit(0)');const light=await execDone(lightId,30000);const lightMs=Math.round(performance.now()-lt);
  let maxRunning=0,maxQueued=0;const t0=performance.now();
  while(true){const states=(await Promise.all(ids.map(execRow))).map(execState);maxRunning=Math.max(maxRunning,states.filter(s=>s==='running'||s==='starting').length);maxQueued=Math.max(maxQueued,states.filter(s=>s!=='running'&&s!=='starting'&&s!=='finished').length);if(states.every(s=>s==='finished'))break;assert(performance.now()-t0<300000,'A4 deadline');await delay(200)}
  out.cells.A4={same_pid:true,max_running:maxRunning,max_queued:maxQueued,light_ms:lightMs,light_state:execState(light),peaks_after_reload:(peakOf()??[]).slice(0,3).map(m=>+(m/GiB).toFixed(2))};save();
 }
 // A5: with #1725's net present, each decision has one owner: no pid is both held/paused by admission and acted on by exhaustion.
 if(cell.includes('A5')){
  const ex=new Set(exhaustionLog().map(r=>r.pid));
  const adm=admissionLog(/./).filter(r=>r.decision==='paused'||r.decision==='held');
  const both=adm.filter(r=>ex.has(r.pid)).map(r=>({pid:r.pid,decision:r.decision}));
  out.cells.A5={exhaustion_pids:[...ex],admission_paused_or_held:adm.length,double_action:both};
  assert.equal(both.length,0,'no command is acted on by both policies');save();
 }
 sig.done=true;await pulsing;out.relay_all=pulse;
 out.admission_lines=admissionLog(/./).map(r=>r.message);
 out.passed=true;save();
} catch(error){out.error=String(error.message);save();throw error}
finally{clearInterval(thrash);clearInterval(beat);await stop();try{cpSync(join(root,'logs'),join(evidence,'logs'),{recursive:true})}catch{}rmSync(root,{recursive:true,force:true})}
