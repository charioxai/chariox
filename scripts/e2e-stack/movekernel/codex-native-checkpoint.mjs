// MP-08 / MP-09 / MP-10 / MP-11: owned real base kernels and product account transfer.
import {openTui} from '../context-switch/tui.mjs';
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js';
import {mkdir,writeFile,readFile,readdir,rm} from 'node:fs/promises';
import {spawn,execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {createWriteStream,createReadStream,readFileSync,readdirSync} from 'node:fs';
import {randomBytes,createHash} from 'node:crypto';
import net from 'node:net';
import {gzipSync} from 'node:zlib';
const repo=path.resolve(fileURLToPath(new URL('../../../',import.meta.url)));
function required(name){const value=process.env[name];if(!value||!path.isAbsolute(value))throw Error('Set an absolute '+name);return value;}
const kernel=required('CHARIOX_CHECKPOINT_KERNEL'),relay=required('CHARIOX_CHECKPOINT_RELAY'),tuiBinary=required('CHARIOX_CHECKPOINT_TUI'),testBinary=required('CHARIOX_CHECKPOINT_TEST_BINARY');
const root=required('CHARIOX_CHECKPOINT_STATE'),evidence=required('CHARIOX_CHECKPOINT_EVIDENCE'),profile=required('CHARIOX_CHECKPOINT_SOURCE_PROFILE');
if(root===repo||root.startsWith(repo+path.sep)||evidence===repo||evidence.startsWith(repo+path.sep))throw Error('State and evidence must be outside the repository');
const protocol=Number(execFileSync(kernel,['--print-local-daemon-protocol-version'],{encoding:'utf8'}).trim());
if(protocol<601)throw Error('Required native resume needs kernel protocol 601');
const head=execFileSync('git',['rev-parse','HEAD'],{cwd:repo,encoding:'utf8'}).trim();
const summary={stateRoot:root,MP:['MP-08','MP-09','MP-10','MP-11'],head,protocol,scope:'Codex Chariox Linux/Linux native adapter; no Move orchestration',started:new Date().toISOString(),processes:[],steps:[]};
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const save=()=>writeFile(evidence+'/summary.json',JSON.stringify(summary,null,2)+'\n',{mode:0o600});
const children=[],clients=[],tuis=[];let sourceRun,sourceStack,marker,ownsRoot=false,driverSerial=0,checkpointArtifact;const owned=new Map();
async function port(){const s=net.createServer();await new Promise((r,j)=>{s.once('error',j);s.listen(0,'127.0.0.1',r)});const p=s.address().port;await new Promise(r=>s.close(r));return p;}
function start(binary,env,label){const c=spawn(binary,[],{env,cwd:root,stdio:['ignore','pipe','pipe']});children.push(c);c.charioxStart=processes().find(p=>p.pid===c.pid)?.start;summary.processes.push({pid:c.pid,label});const log=createWriteStream(root+'/'+label+'.private.log',{mode:0o600});c.stdout.pipe(log);c.stderr.pipe(log);return c;}
async function stop(c){if(c.exitCode!==null||c.signalCode!==null)return;const exited=new Promise(r=>c.once('exit',r));if(!Number.isInteger(c.pid)||c.pid<=1||!processes().some(p=>p.pid===c.pid&&p.start===c.charioxStart))throw Error('Invalid owned PID');c.kill('SIGINT');await Promise.race([exited,sleep(15000)]);if(c.exitCode===null&&c.signalCode===null){c.kill('SIGKILL');await exited;}}
function processes(){const rows=[];for(const name of readdirSync('/proc')){if(!/^\d+$/.test(name))continue;try{const text=readFileSync('/proc/'+name+'/stat','utf8'),rest=text.slice(text.lastIndexOf(')')+2).split(' ');rows.push({pid:Number(name),state:rest[0],ppid:Number(rest[1]),start:rest[19]});}catch{}}return rows;}
async function binaryHash(binary){const hash=createHash('sha256');for await(const chunk of createReadStream(binary))hash.update(chunk);return hash.digest('hex');}

function rememberOwned(){const rows=processes(),ids=new Set([...children,...tuis.map(t=>t.child)].filter(c=>c.exitCode===null&&c.signalCode===null&&rows.some(p=>p.pid===c.pid&&p.start===c.charioxStart)).map(c=>c.pid));let changed=true;while(changed){changed=false;for(const row of rows)if(ids.has(row.ppid)&&!ids.has(row.pid)){ids.add(row.pid);changed=true;}}for(const row of rows)if(ids.has(row.pid))owned.set(row.pid,row);}
async function adapter(operation,stack,providerRun,codexHome){const serial=++driverSerial,input=root+'/driver-'+serial+'.json',output=root+'/driver-'+serial+'-output.json';if(operation==='export')checkpointArtifact=root+'/native-checkpoint-'+serial+'.private.json';if(operation==='import'&&!checkpointArtifact)throw Error('Export a checkpoint before importing');await writeFile(input,JSON.stringify({operation,provider_run:providerRun,codex_home:codexHome,artifact:checkpointArtifact??root+'/unused.private.json',output}),{mode:0o600});const child=spawn(testBinary,['--exact','provider::checkpoint::live::live_codex_native_checkpoint_driver','--ignored','--nocapture'],{cwd:repo,env:{...process.env,...stack.env,CHARIOX_NATIVE_CHECKPOINT_DRILL:input},stdio:['ignore','pipe','pipe']});children.push(child);child.charioxStart=processes().find(p=>p.pid===child.pid)?.start;const timer=setTimeout(()=>{rememberOwned();if(Number.isInteger(child.pid)&&child.pid>1&&processes().some(p=>p.pid===child.pid&&p.start===child.charioxStart))child.kill('SIGTERM');},240000);const stream=createWriteStream(evidence+'/adapter-'+operation+'-'+serial+'.log',{mode:0o600});child.stdout.pipe(stream);child.stderr.pipe(stream);const rc=await new Promise((resolve,reject)=>{child.once('error',reject);child.once('exit',resolve)});clearTimeout(timer);if(rc!==0)throw Error('Native adapter '+operation+' failed; inspect its redacted test log');return JSON.parse(await readFile(output,'utf8'));}
async function terminate(stack,providerRun){const before=(await request(stack.client,{ListProviderProcesses:{provider:'codex'}},'ProviderProcessesListed')).processes;const ownedPids=before.filter(p=>(p.owner_provider_run_ids??[]).includes(providerRun.id)).map(p=>p.pid).filter(Number.isInteger);await stack.client.send({TeardownProviderProcesses:{provider:'codex',force:true}});let ended;for(let n=0;n<80;n++){ended=(await request(stack.client,{GetProviderRun:{provider_run_id:providerRun.id}},'ProviderRun')).provider_run;const live=processes().some(p=>ownedPids.includes(p.pid)&&p.state!=='Z');if(ended.state==='Ended'&&!live)return ended;await sleep(250)}throw Error('Source provider teardown was not confirmed');}
async function request(c,req,key){const result=await c.send(req);if(!result[key])throw Error('Unexpected response for '+Object.keys(req)[0]+': '+Object.keys(result).join(','));return result[key];}
async function launch(stack,fields){const result=await stack.client.send({LaunchProviderRun:fields}),accepted=result.ProviderRunLaunched??result.ProviderRunLaunchAccepted;if(!accepted?.provider_run?.id)throw Error('Unexpected provider launch response: '+Object.keys(result).join(','));for(let n=0;n<240;n++){const run=(await request(stack.client,{GetProviderRun:{provider_run_id:accepted.provider_run.id}},'ProviderRun')).provider_run;if(run.state==='Running'&&run.structured_endpoint)return {provider_run:run};if(run.state==='Ended')throw Error('Provider launch ended before runtime readiness');await sleep(250)}throw Error('Provider launch readiness timeout');}
try{
 await mkdir(evidence,{recursive:true,mode:0o700});
 await mkdir(root,{recursive:false,mode:0o700});ownsRoot=true;
 summary.binarySha256={};for(const [name,binary] of Object.entries({kernel,relay,tui:tuiBinary,driver:testBinary}))summary.binarySha256[name]=await binaryHash(binary);
 const relayPort=await port(),secret=randomBytes(32).toString('base64url');
 start(relay,{PATH:process.env.PATH,CHARIOX_RELAY_HOST:'127.0.0.1',CHARIOX_RELAY_PORT:String(relayPort),CHARIOX_RELAY_TOKEN:secret},'relay');
 const stacks=[];
 for(const name of ['source','target']){
  const dir=root+'/'+name;for(const d of ['home','os-home','tmp','workspace'])await mkdir(dir+'/'+d,{recursive:true,mode:0o700});
  await writeFile(dir+'/home/config.toml','version = 1\n[history.archive]\nmode = "disabled"\n',{mode:0o600});
  const p=await port();const env={PATH:process.env.PATH,LANG:'C.UTF-8',HOME:dir+'/os-home',CHARIOX_HOME:dir+'/home',TMPDIR:dir+'/tmp',CHARIOX_KERNEL_HOST:'127.0.0.1',CHARIOX_KERNEL_PORT:String(p),CHARIOX_MCP_PORT:String(await port()),CHARIOX_CODEX_PORT:String(await port()),CHARIOX_OPENCODE_PORT:String(await port()),CHARIOX_RELAY_URL:'ws://127.0.0.1:'+relayPort,CHARIOX_RELAY_TOKEN:secret,CHARIOX_DAEMON_SOCKET:dir+'/daemon.sock',CHARIOX_KERNEL_RUNTIME_ROLE:'general',CHARIOX_ACCEPT_REMOTE_LEASES:'1',CHARIOX_REMOTE_LEASE_CAPACITY:'2',NODE_OPTIONS:'--max-old-space-size=4096',RUST_MIN_STACK:'16777216'};
  const child=start(kernel,env,name);let client;
  for(let n=0;n<80;n++){if(child.exitCode!==null)throw Error(name+' kernel exited '+child.exitCode);const c=new LocalIpcClient('ws://127.0.0.1:'+p+'/kernel',{timeoutMs:30000,localAuthEnvironment:env});try{await c.send({ListSessions:null});client=c;clients.push(c);break}catch{await c.close().catch(()=>{});await sleep(500)}}
  if(!client)throw Error(name+' kernel readiness timeout');
  let status;for(let n=0;n<40;n++){status=(await request(client,{RelayStatus:null},'RelayStatus')).status;if(status.connected)break;await sleep(500)}
  summary.steps.push({name,ready:true,connected:status.connected,kernel:status.daemon_id,machine:status.machine_id});await save();
  stacks.push({name,dir,env,client,status});
 }
 const [source,target]=stacks;sourceStack=source;
 const readme=await readFile(repo+'/README.md');await writeFile(source.dir+'/workspace/README.md',readme);await writeFile(target.dir+'/workspace/README.md',readme);
 const linked=(await request(source.client,{LinkProviderAccountProfile:{provider:'codex',label:'Move proof',path:profile}},'ProviderAccountProfile')).profile;
 await source.client.send({RefreshProviderAccountProfile:{provider:'codex',account_profile:linked.profile_id}});
 summary.steps.push({linked:true,profile:linked.profile_id});await save();
 let machines=[];for(let n=0;n<40;n++){const response=await source.client.send({ListRemoteMachines:null});machines=response.RemoteMachinesListed?.machines??[];if(machines.some(m=>m.machine_id===target.status.machine_id))break;await sleep(500)}
 summary.steps.push({machineFields:machines.map(m=>({keys:Object.keys(m),machine_id:m.machine_id,kernel_id:m.kernel_id,approval:m.approval}))});await save();
 const found=machines.find(m=>m.machine_id===target.status.machine_id);if(!found)throw Error('Owned target not discovered over local relay');
 await source.client.send({ApproveRemoteMachine:{machine_ref:target.status.machine_id}});
 const created=(await request(source.client,{CreateSession:{workspace_id:source.dir+'/workspace',worktree_id:source.dir+'/workspace',alias:'Move proof',agent_defaults:{provider:'codex',account_profile:linked.profile_id,model:'gpt-6-luna',effort:'low',permission_level:'required'}}},'SessionCreated'));
 const spawned=await source.client.send({SpawnAgent:{session_id:created.session.id,alias:'Remote proof',provider:'codex',account_profile:linked.profile_id,model:'gpt-6-luna',effort:'low',permission_level:'required',worktree_id:target.dir+'/workspace',kernel_ref:target.status.daemon_id}});
 summary.steps.push({spawnResponse:Object.keys(spawned),remoteBound:!!spawned.AgentSpawned?.agent?.remote_execution});await save();
 let list=(await request(target.client,{ListProviderAccountProfiles:{provider:'codex'}},'ProviderAccountProfilesListed')).profiles;
 summary.steps.push({initialTargetProfiles:list.map(p=>({id:p.profile_id,state:p.auth_state}))});await save();
 if(!list.some(p=>p.profile_id===linked.profile_id)){
  const attached=await request(source.client,{AttachToSession:{session_id:created.session.id,client_id:'movekernel-proof',capability_level:'FullTerminal'}},'SessionAttached');
  const remoteId=spawned.AgentSpawned?.agent?.id;if(!remoteId)throw Error('Missing remote agent identity');
  await source.client.send({SubmitPrompt:{session_id:created.session.id,attachment_id:attached.attachment.id,target_agent_id:remoteId,prompt:'Reply only READY. Do not use tools or create agents.',attachments:[]}});
  for(let n=0;n<180;n++){list=(await request(target.client,{ListProviderAccountProfiles:{provider:'codex'}},'ProviderAccountProfilesListed')).profiles;const state=(await request(source.client,{GetSessionState:{session_id:created.session.id}},'SessionState')).session;const agent=state.agents.find(a=>a.id===remoteId);if(list.some(p=>p.profile_id===linked.profile_id)&&agent&&!agent.is_processing&&agent.state!=='Working'){summary.steps.push({remotePromptSettled:true,state:agent.state,model:agent.model});break}await sleep(500)}
 }
 list=list.filter(p=>p.profile_id===linked.profile_id);if(list.length!==1)throw Error('Target account replica not uniquely installed after preparation');
 const folders=await readdir(target.dir+'/home/state/provider-accounts');
 const owner=folders.find(v=>v!=='local')??'local';
 const targetHome=target.dir+'/home/state/provider-accounts/'+owner+'/codex/'+list[0].profile_id+'/codex';
 summary.targetProfile=targetHome;summary.steps.push({accountTransferred:true,targetProfileId:list[0].profile_id});await save();
 await writeFile(evidence+'/kernel-target-selector.json',JSON.stringify({CODEX_HOME:targetHome})+'\n',{mode:0o600});
 async function command(tui,text){await tui.type('\x15/'+text+'\r');await sleep(700);}
 async function focus(tui){const s=await tui.send('snapshot');return s.session.focusedAgentId;}
 function entries(snapshot){return snapshot.agentPanes?.[snapshot.session.focusedAgentId]??[];}
 async function turn(tui,prompt){const before=new Set(entries(await tui.send('snapshot')).map(r=>r.id));await tui.send('submit_prompt',{prompt});let saw=false,settledAt;for(let n=0;n<240;n++){const s=await tui.send('snapshot'),a=s.session.agents.find(a=>a.id===s.session.focusedAgentId),busy=a?.isProcessing||a?.state==='Working';if(busy){saw=true;settledAt=undefined;}const terminal=entries(s).some(r=>!before.has(r.id)&&['assistant','error'].includes(r.role)&&(r.text??'').trim());if(a&&!busy&&(saw||terminal)){settledAt??=Date.now();if(Date.now()-settledAt>=1500)return {snapshot:s,sawWorking:saw};}await sleep(250)}throw Error('TUI turn deadline');}

 async function run(stack,tui){const s=await tui.send('snapshot');const core=(await request(stack.client,{GetSessionState:{session_id:s.session.id}},'SessionState'));const a=core.session.agents.find(a=>a.id===s.session.focusedAgentId);const id=a.active_provider_run_id??core.agent_activity?.[a.id]?.last_completed_turn?.provider_run_id;if(!id)throw Error('No real provider run identity');return (await request(stack.client,{GetProviderRun:{provider_run_id:id}},'ProviderRun')).provider_run;}
 const sourceTui=await openTui({tree:source.dir+'/workspace',env:source.env,url:'ws://127.0.0.1:'+source.env.CHARIOX_KERNEL_PORT+'/kernel',profile:linked.profile_id,root:source.dir,provider:'codex',model:'gpt-6-luna',binary:tuiBinary});sourceTui.child.charioxStart=processes().find(p=>p.pid===sourceTui.child.pid)?.start;tuis.push(sourceTui);
 await command(sourceTui,'permissions required');await command(sourceTui,'agent variant '+await focus(sourceTui)+' low');
 marker='orchid-'+randomBytes(12).toString('hex');
 const sourceTurn=await turn(sourceTui,'Remember this marker for our next task: '+marker+'. Reply only ACK. Do not use tools or create agents.');
 if(sourceTurn.snapshot.session.agents.find(a=>a.id===sourceTurn.snapshot.session.focusedAgentId)?.state==='Error'||!entries(sourceTurn.snapshot).some(r=>r.role==='assistant'&&(r.text??'').includes('ACK')))throw Error('The real source provider turn did not succeed');
 sourceRun=await run(source,sourceTui);if(!sourceRun.provider_session_id)throw Error('Source has no real native thread');
 summary.steps.push({sourceRealTurn:true,model:sourceRun.model,variant:sourceRun.variant});await save();
 const homeBefore=sourceTurn.snapshot.session;
 try{await source.client.send({MoveAgentToRemote:{session_id:homeBefore.id,agent_ref:homeBefore.focusedAgentId,machine_ref:target.status.machine_id}});throw Error('Unexpected lease move admission before checkpoint composition');}
 catch(error){if(!String(error.message).includes('agent has a provider run; stop or destroy the provider run before moving it'))throw error;summary.leaseMoveProbe={admitted:false,reason:'Existing MoveAgentToRemote refuses agents with provider runs; checkpoint lease composition belongs to later Move PRs.',sameHomeSession:true};await save();}
 const session=(await request(target.client,{CreateSession:{workspace_id:target.dir+'/workspace',worktree_id:target.dir+'/workspace',alias:'Target resume proof',agent_defaults:{provider:'codex',account_profile:linked.profile_id,model:'gpt-6-luna',effort:'low',permission_level:'required'}}},'SessionCreated'));
 const endedSource=await terminate(source,sourceRun);
 const exported=await adapter('export',source,endedSource,profile);
 await adapter('delete',source,endedSource,profile);
 summary.sourceNativeSessionRemoved=true;
 const binding=exported.binding;
 const launched=await launch(target,{session_id:session.session.id,agent_id:session.agent.id,adapter_key:'codex',provider:'codex',account_profile:linked.profile_id,model:'gpt-6-luna',variant:'low',provider_session_id:sourceRun.provider_session_id,native_tui:false,native_checkpoint_binding:binding});
 await adapter('import',target,launched.provider_run,targetHome);
 summary.steps.push({sourceStopped:true,checkpointExported:true,checkpointImported:true,sourceAndTargetProfilesDiffer:profile!==targetHome});await save();
 const targetTui=await openTui({tree:target.dir+'/workspace',env:target.env,url:'ws://127.0.0.1:'+target.env.CHARIOX_KERNEL_PORT+'/kernel',profile:linked.profile_id,session:session.session.id,root:target.dir,provider:'codex',model:'gpt-6-luna',binary:tuiBinary});targetTui.child.charioxStart=processes().find(p=>p.pid===targetTui.child.pid)?.start;tuis.push(targetTui);
 const targetPrompt='Use a shell command to print your working directory, then repeat the marker from our earlier conversation. Do not use any other tools.';
 if(targetPrompt.includes(marker))throw Error('Drill must not send the earlier marker to the target');
 const first=await turn(targetTui,targetPrompt);
 const rendered=JSON.stringify(entries(first.snapshot)),targetRun=await run(target,targetTui);
 const positive={sameNativeId:targetRun.provider_session_id===sourceRun.provider_session_id,contextRecalled:entries(first.snapshot).some(row=>row.role==='assistant'&&(row.text??'').includes(marker)),relocatedWorkspaceExecuted:rendered.includes(target.dir+'/workspace'),model:targetRun.model,variant:targetRun.variant,state:first.snapshot.session.agents.find(a=>a.id===first.snapshot.session.focusedAgentId)?.state,observedRoles:[...new Set(entries(first.snapshot).map(row=>row.role))]};
 summary.positive=positive;await save();
 if(!positive.sameNativeId||!positive.contextRecalled||!positive.relocatedWorkspaceExecuted||positive.state==='Error')throw Error('Native continuity or relocated workspace failed');
 const endedTarget=await terminate(target,targetRun);
 const returnCheckpoint=await adapter('export',target,endedTarget,targetHome);
 const returned=await launch(source,{session_id:homeBefore.id,agent_id:homeBefore.focusedAgentId,adapter_key:'codex',provider:'codex',account_profile:linked.profile_id,model:'gpt-6-luna',variant:'low',provider_session_id:sourceRun.provider_session_id,native_tui:false,native_checkpoint_binding:returnCheckpoint.binding});
 summary.sourceNativeSessionRemoved=false;
 await adapter('import',source,returned.provider_run,profile);
 const back=await turn(sourceTui,'Use a shell command to print your working directory, then repeat the marker and the working directory you printed in the previous task. Do not use any other tools.');
 const backRun=await run(source,sourceTui),backEntries=entries(back.snapshot),backText=JSON.stringify(backEntries);
 summary.returned={sameNativeId:backRun.provider_session_id===sourceRun.provider_session_id,contextRecalled:backEntries.some(row=>row.role==='assistant'&&(row.text??'').includes(marker)),targetTurnRecalled:backEntries.some(row=>row.role==='assistant'&&(row.text??'').includes(target.dir+'/workspace')),homeWorkspaceExecuted:backText.includes(source.dir+'/workspace'),sameHomeSession:back.snapshot.session.id===homeBefore.id,sameHomeAgent:back.snapshot.session.focusedAgentId===homeBefore.focusedAgentId,homeHistoryRetained:backEntries.some(row=>row.role==='assistant'&&(row.text??'').includes('ACK')),providerSucceeded:back.snapshot.session.agents.find(a=>a.id===back.snapshot.session.focusedAgentId)?.state!=='Error'};await save();
 if(Object.values(summary.returned).some(value=>value!==true))throw Error('Return checkpoint continuity, workspace or home history failed');
 await terminate(source,backRun);
 await adapter('delete',target,endedTarget,targetHome);
 const nativeBeforeFailure=new Set(await readdir(targetHome+'/sessions',{recursive:true}).catch(()=>[]));
 await launch(target,{session_id:session.session.id,agent_id:session.agent.id,adapter_key:'codex',provider:'codex',account_profile:linked.profile_id,model:'gpt-6-luna',variant:'low',provider_session_id:sourceRun.provider_session_id,native_tui:false,native_checkpoint_binding:binding});
 const failed=await turn(targetTui,'Reply only ACK. Do not use tools or create agents.');
 const failedAgain=await turn(targetTui,'Reply only ACK. Do not use tools or create agents.');
 const retained=await run(target,targetTui);
 const files=await readdir(targetHome+'/sessions',{recursive:true}).catch(()=>[]);
 summary.negative={firstError:failed.snapshot.session.agents.find(a=>a.id===failed.snapshot.session.focusedAgentId)?.state==='Error',secondError:failedAgain.snapshot.session.agents.find(a=>a.id===failedAgain.snapshot.session.focusedAgentId)?.state==='Error',sameNativeIdRetained:retained.provider_session_id===sourceRun.provider_session_id,noFreshNativeArtifacts:!files.some(name=>name.endsWith('.jsonl')&&!nativeBeforeFailure.has(name))};await save();
 if(Object.values(summary.negative).some(value=>value!==true))throw Error('Required resume silently recovered or lost its native identity');
 summary.passed=true;


}catch(e){summary.error=String(e.message).slice(0,400);summary.passed=false;}
finally{
 rememberOwned();
 for(const t of tuis)await t.close().catch(()=>{});
 for(const c of clients)try{const list=await c.send({ListSessions:null});for(const s of list.SessionsListed?.sessions??[])await c.send({DeleteSession:{session_id:s.id}});await c.send({TeardownProviderProcesses:{provider:null,force:true}});}catch{}
 for(const c of clients)await c.close().catch(()=>{});
 for(const c of children.reverse())await stop(c);
 for(let n=0;n<20;n++){const left=processes().filter(p=>owned.has(p.pid)&&owned.get(p.pid).start===p.start&&p.state!=='Z');if(!left.length)break;await sleep(250);}
 const left=processes().filter(p=>owned.has(p.pid)&&owned.get(p.pid).start===p.start&&p.state!=='Z');
 for(const p of left){if(!Number.isInteger(p.pid)||p.pid<=1)throw Error('Invalid owned cleanup PID');process.kill(p.pid,'SIGTERM');}
 if(left.length)await sleep(1000);
 const remaining=processes().filter(p=>owned.has(p.pid)&&owned.get(p.pid).start===p.start&&p.state!=='Z');
 for(const p of remaining){if(!Number.isInteger(p.pid)||p.pid<=1)throw Error('Invalid owned cleanup PID');process.kill(p.pid,'SIGKILL');}
 if(sourceRun&&sourceStack&&summary.sourceNativeSessionRemoved!==true)try{await adapter('delete',sourceStack,sourceRun,profile);summary.sourceNativeSessionRemoved=true;}catch{summary.sourceNativeSessionRemoved=false;summary.passed=false;}
 summary.cleanup={remainingOwnedProcesses:processes().filter(p=>owned.has(p.pid)&&owned.get(p.pid).start===p.start&&p.state!=='Z').map(p=>p.pid)};
 if(summary.cleanup.remainingOwnedProcesses.length)summary.passed=false;
 for(const name of ['source','target'])try{const raw=await readFile(root+'/'+name+'/tui.ansi','utf8'),safe=marker?raw.split(marker).join('<native-context-marker>'):raw;const plain=safe.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g,'');if(/Bearer\s+[A-Za-z0-9._-]{12,}|sk-[A-Za-z0-9_-]{20,}|[A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{20,}|-----BEGIN.*PRIVATE KEY/.test(plain)){summary.cleanup.captureRefused=true;}else await writeFile(evidence+'/'+name+'-tui.ansi.gz',gzipSync(safe),{mode:0o600});}catch{}
 if(ownsRoot&&summary.sourceNativeSessionRemoved!==false){await rm(root,{recursive:true,force:true});summary.cleanup.stateRemoved=true;}
 summary.finished=new Date().toISOString();await save();console.log(JSON.stringify(summary,null,2));
 if(!summary.passed)process.exitCode=1;
}
