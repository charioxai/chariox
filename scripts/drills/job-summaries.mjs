// Real local API retained-job load drill. Run only on the explicitly named remote drill host.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdirSync,writeFileSync,openSync,rmSync,statSync,readFileSync} from 'node:fs';
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
writeFileSync(join(root,'chariox/config.toml'),`[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${root}/chariox/state.db"\n`,{mode:0o600});
const env={PATH:process.env.PATH,HOME:join(root,'home'),TMPDIR:join(root,'tmp'),IS_SANDBOX:'1',SHELL:'/bin/bash',CHARIOX_PROVIDER_DEV_STUB:'1',CHARIOX_HOME:join(root,'chariox'),CHARIOX_LOG_DIR:join(root,'logs'),CHARIOX_SESSION_HISTORY_DIR:join(root,'history'),CHARIOX_DAEMON_SOCKET:join('/var/tmp/jobsview',label+'.sock'),CHARIOX_DAEMON_ALIAS:'jobsview-'+label,CHARIOX_MACHINE_ALIAS:'jobsview-'+label,CHARIOX_KERNEL_PORT:String(ports[0]),CHARIOX_MCP_PORT:String(ports[1]),CHARIOX_OPENCODE_PORT:String(ports[2]),CHARIOX_CODEX_PORT:String(ports[3]),CHARIOX_KERNEL_BROWSER_PORT:String(ports[4]),XDG_CONFIG_HOME:join(root,'config'),XDG_CACHE_HOME:join(root,'cache'),XDG_STATE_HOME:join(root,'state')};
let child,client;const clients=[];const out={label,mode,at:new Date().toISOString(),retained:2000,subscribers:16,errors:[],binary_sha256:createHash('sha256').update(readFileSync(binary)).digest('hex'),head:flags.get('--head')};
const save=()=>writeFileSync(join(evidence,'measurements.json'),JSON.stringify(out,null,2)+'\n');
const log=openSync(join(evidence,'kernel.log'),'a',0o600);
async function bounded(p,label,ms=120000){let t;try{return await Promise.race([p,new Promise((_,reject)=>t=setTimeout(()=>reject(Error(label+' deadline')),ms))])}finally{clearTimeout(t)}}
async function start(){child=spawn(binary,[],{env,stdio:['ignore',log,log]});client=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:env,controlRequestRetryDeadlineMs:1000});for(let i=0;i<120;i++){assert(child.exitCode===null&&child.signalCode===null,'kernel exited');try{if((await client.send({RelayStatus:null})).RelayStatus)return}catch{}await delay(250)}throw Error('startup deadline')}
async function stop(){await client?.close();if(child?.exitCode===null&&child.signalCode===null){assert(Number.isSafeInteger(child.pid)&&child.pid>1);child.kill('SIGTERM');for(let i=0;i<100&&child.exitCode===null&&child.signalCode===null;i++)await delay(100);if(child.exitCode===null&&child.signalCode===null)child.kill('SIGKILL');await bounded(new Promise(r=>child.exitCode!==null||child.signalCode!==null?r():child.once('exit',r)),'stop',10000)}}
const rpc=async request=>{const value=await bounded(client.send(request),Object.keys(request)[0]);if(value.Error)throw Error(value.Error.message);return value};
const dist=a=>{a.sort((a,b)=>a-b);return{n:a.length,p50_ms:a[Math.max(0,Math.ceil(a.length*.5)-1)],p99_ms:a[Math.max(0,Math.ceil(a.length*.99)-1)],max_ms:a.at(-1)}};
try{
 await start();
 const made=(await rpc({CreateSession:{workspace_id:join(root,'work'),worktree_id:join(root,'work'),alias:'jobsview',agent_defaults:{provider:'dev-stub',account_profile:'default',model:'runtime-mcp-fixture',permission_level:'yolo'}}})).SessionCreated;
 const room=made.session.id,agent=made.agent.id;
 let att=(await rpc({AttachToSession:{session_id:room,client_id:'jobsview-owner',capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
 await stop();
 const dir=join(root,'chariox/jobs');mkdirSync(dir,{recursive:true});
 const records=[];
 for(let i=0;i<2000;i++){
  const id='job-'+i.toString(16).padStart(32,'0');const same=i<64;
  const record={job_id:id,wake_id:'seed-wake-'+i,registration_id:'seed-registration-'+i,room_id:same?room:'seed-other-room',agent_id:agent,request:{command:['/bin/true'],cwd:null,env:[],needs:{},timeout_s:60,wait_s:0,outputs:[]},workspace:join(root,'work'),cwd:join(root,'work'),placement:'local',started_at_ms:100000+i,running_at_ms:100000+i,state:'finished',allocation_ref:null,capability_revision:null,machine_label:null,address:null,result:{exit_code:0,output_tail:'retained output '+('x'.repeat(65500)),log:'synthetic-job.log',files_back:['file.txt'],conflicts:[],ran_on:{machine:'this machine',ip:null},duration_ms:1000,outcome:'completed'},exec_id:null,class_id:null,rate_per_minute:null,estimate_micros:null,deleted:false,remote_available:false,remote_refusal:null,estimated_minutes:null,memory_mib:null,remote:'Idle'};
  const body=join(dir,id+'.json'), bytes=JSON.stringify(record);
  records.push(record);writeFileSync(body,bytes);
  const directory=join(dir,'summaries',createHash('sha256').update(record.room_id).digest('hex'),agent);
  mkdirSync(directory,{recursive:true});
  const view={job_id:id,agent_id:agent,command:'/bin/true',state:'finished',placement:'local',started_at_ms:record.started_at_ms,machine_label:null,address:null,class_id:null,rate_per_minute:null,estimate_micros:null,deleted:false,result:{exit_code:0,reason:null,files_back_count:1,conflicts_count:0,ran_on:record.result.ran_on,duration_ms:1000,outcome:'completed'},read_error:null};
  writeFileSync(join(directory,id+'.json'),JSON.stringify({view,workspace:record.workspace,cwd:record.cwd,command_hash:createHash('sha256').update(JSON.stringify(record.request.command)).digest('hex'),record_bytes:Buffer.byteLength(bytes),record_modified_ns:statSync(body,{bigint:true}).mtimeNs.toString()}));
 }

 await start();
 att=(await rpc({AttachToSession:{session_id:room,client_id:'jobsview-owner-restarted',capability_level:'FullTerminal'}})).SessionAttached.attachment.id;
 const readers=[];
 for(let i=0;i<16;i++){const c=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:env});clients.push(c);const a=(await c.send({AttachToSession:{session_id:room,client_id:'subscriber-'+i,capability_level:'FullTerminal'}})).SessionAttached.attachment.id;await c.subscribeToKernelEvents(room,a);readers.push(c)}
 const list=[];for(let i=0;i<50;i++){const at=performance.now();const v=await rpc({GetSessionState:{session_id:room}});list.push(performance.now()-at);assert.equal(v.SessionState.jobs.length,64)}
 out.refresh=dist(list);save();
 const launch=await rpc(requests.launchProviderRunsRequest([{sessionId:room,provider:'dev-stub',accountProfile:'default',effort:'low',model:'runtime-mcp-fixture',agentId:agent,native:{nativeTui:false}}],1));
 let run=launch.ProviderRunsLaunchAccepted.provider_runs[0].provider_run;for(let i=0;i<120;i++){const r=Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0];const p=r.provider_run??r;if(p.state==='Running'){run=p;break}assert(!['Stopped','Failed','Exited'].includes(p.state),'provider fixture ended during launch');await delay(250)}
 const fx=bindProviderMcpFixture(run,{client,requests,sessionId:room,attachmentId:att});
 await delay(1500);await rpc(requests.submitPromptRequest(room,att,agent,'go',[]));await delay(1000);
 const session=(await rpc({GetSessionState:{session_id:room}})).SessionState;
 const tasks=session.agent_tasks??session.session.agent_tasks??[];out.task_fields=Object.keys(tasks[0]??{});save();
 const task=tasks.find(t=>t.agent_id===agent&&t.state==='working')??tasks.find(t=>t.agent_id===agent);assert(task,'running task');
 let origin=task.prompt_id;
 const invoke=async(name,args)=>{
  let settled=false;
  const pending=fx.fixtureMcp.call('tools/call',{name,arguments:args},{timeoutMs:120000,rawEnvelope:true}).finally(()=>{settled=true});
  const consent=(async()=>{
   while(!settled){
    assert(child.exitCode===null&&child.signalCode===null,'kernel exited during admission');
    const current=Object.values(await rpc(requests.getProviderRunRequest(run.id)))[0];assert((current.provider_run??current).state==='Running','provider fixture exited during MCP request');
    const state=(await rpc({GetSessionState:{session_id:room}})).SessionState;
    for(const interaction of state.session.active_interactions??[]){
     if(interaction.agent_id!==agent||!interaction.id.startsWith('watched-process-'))continue;
     assert(interaction.choices.some(choice=>choice.id==='allow_agent'),'owned synthetic command consent');
     await rpc({RespondToInteraction:{session_id:room,interaction_id:interaction.id,choice_id:'allow_agent'}});
     out.synthetic_command_consents=(out.synthetic_command_consents??0)+1;
    }
    if(!settled)await delay(250);
   }
  })();
  try{return await Promise.race([pending,consent.then(()=>pending)])}finally{settled=true;await consent}
 };
 const bridge=await fx.fixtureMcp.call('fixture/input-digest',{}, {timeoutMs:10000});
 assert(bridge&&Object.hasOwn(bridge,'sha256'),'private MCP fixture responds before admission');
 const tool=async(name,args)=>{
  for(let attempt=0;attempt<3;attempt++){
   const res=await invoke(name,{...args,task_id:task.task_id,origin_prompt_id:origin});
   if(res.error){
    const next=res.error.message?.match(/origin_prompt_id is `([^`]+)`/);
    if(next){origin=next[1];continue}
    throw Error(res.error.message)
   }
   const value=res.result??res;assert(!value.isError,JSON.stringify(value.content).slice(0,250));
   return value.structuredContent??JSON.parse(value.content.find(c=>c.type==='text').text);
  }
  throw Error('turn authority changed repeatedly');
 };
 const admission=[];out.admission_attempts=[];
 for(let i=0;i<8;i++){
  const refresh=rpc({GetSessionState:{session_id:room}});const at=performance.now();out.admission_attempts.push({started_at:new Date().toISOString(),deadline_ms:120000});save();const job=await tool('chariox.jobs.run',{command:['/bin/true'],wait_s:0,task_id:'unused',origin_prompt_id:'unused'});admission.push(performance.now()-at);out.admission_attempts.at(-1).elapsed_ms=admission.at(-1);save();assert(job.job_id);await refresh;
 }
 out.admission=dist(admission);
 const opened=await tool('chariox.jobs.status',{job_id:records[0].job_id});assert.equal(opened.result.output_tail,records[0].result.output_tail);out.full_result=true;
 // Corruption is injected only into known synthetic files, never read back from kernel state.
 writeFileSync(join(dir,records[1].job_id+'.json'),'x'.repeat(Buffer.byteLength(JSON.stringify(records[1]))));
 rmSync(join(dir,records[2].job_id+'.json'));
 const brokenHeader=join(dir,'summaries',createHash('sha256').update(room).digest('hex'),agent,records[3].job_id+'.json');
 writeFileSync(brokenHeader,'broken header');
 const damage=(await rpc({GetSessionState:{session_id:room}})).SessionState.jobs;
 out.damage=records.slice(1,4).map(record=>({job_id:record.job_id,row:damage.find(job=>job.job_id===record.job_id)??null}));
 if(mode==='head')assert(out.damage.every(row=>row.row?.read_error),'damaged rows stay visible with reasons');
 for(const c of clients)await c.close();clients.length=0;
 const before=damage.map(job=>job.job_id).sort();
 const mapping=()=>readFileSync(`/proc/${child.pid}/maps`,'utf8').split('\n')[0];
 const prior=mapping();assert(Number.isSafeInteger(child.pid)&&child.pid>1);child.kill('SIGUSR2');
 await bounded((async()=>{for(;;){assert(child.exitCode===null&&child.signalCode===null,'kernel exited during reload');if(mapping()!==prior)return;await delay(100)}})(),'hot reload',60000);
 await client.close();client=new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`,{localAuthEnvironment:env,controlRequestRetryDeadlineMs:1000});
 let reloaded;
 for(let i=0;i<120;i++){try{reloaded=(await rpc({GetSessionState:{session_id:room}})).SessionState.jobs;break}catch{assert(child.exitCode===null&&child.signalCode===null,'kernel exited');await delay(250)}}
 assert(reloaded,'reload ready');out.reload_same_list=JSON.stringify(reloaded.map(job=>job.job_id).sort())===JSON.stringify(before);assert(out.reload_same_list,'reload preserves every listed job');
 await stop();await start();
 const restarted=(await rpc({GetSessionState:{session_id:room}})).SessionState.jobs;
 out.restart_same_list=JSON.stringify(restarted.map(job=>job.job_id).sort())===JSON.stringify(before);
 assert(out.restart_same_list,'restart preserves every listed job');
 save();console.log(JSON.stringify(out));
} catch(error){out.errors.push(String(error));save();throw error}
finally{for(const c of clients)await c.close().catch(()=>{});await stop();rmSync(root,{recursive:true,force:true})}
