// WP-12 / MP-08 / MP-10. One bounded task through the ordinary Chariox Room.
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {execFile,spawn} from 'node:child_process';
import {promisify} from 'node:util';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {statfs} from 'node:fs/promises';
import {roomProviderToolName} from '../lib/room-provider-tool-record.mjs';
import {stopOwnedProcess} from './round2/owned-processes.mjs';
import {observeKernelRpcErrors,rpcErrorRecord} from './round2/rpc-errors.mjs';
import {loadTurnHistory,assembleTurnEntries,submitBenchmarkPrompt,getTurn} from './round2/kernel.mjs';
import {sanitizeDrillMetadata} from '../lib/drill-secrets.mjs';
const guard=async()=>{const mem=Number((await readFile('/proc/meminfo','utf8')).match(/MemAvailable:\s+(\d+)/)[1])*1024;const fs=await statfs('/');const disk=fs.bavail*fs.bsize;if(mem<16*1024**3||disk<10*1024**3)throw Error('MP-10 resource floor reached');};
const exec=promisify(execFile), sleep=ms=>new Promise(r=>setTimeout(r,ms));
const config=JSON.parse(await readFile(process.argv[2],'utf8'));
const taskId=process.argv[3];
assert(/^Webmall_[A-Za-z0-9_]+$/.test(taskId),'select one frozen task ID');
const task=JSON.parse(await readFile(config.agentInputs,'utf8')).find(t=>t.task_id===taskId);
assert(task,'task is absent from frozen public inputs');
const {LocalIpcClient}=await import(pathToFileURL(config.clientRoot+'/dist/ipc.js'));
const requests=await import(pathToFileURL(config.clientRoot+'/dist/ipc-requests.js'));
const helpers=await import(pathToFileURL(config.clientRoot+'/dist/session-history-fragments.js'));
const client=new LocalIpcClient(config.kernelUrl,{controlResponseStallMs:240000});
const keepAlive=setInterval(()=>{},1000);
const one=(r,k)=>{assert(r[k],'expected '+k);return r[k]};
const runId=randomUUID().slice(0,8), dir=path.join(config.outputRoot,String(taskId));
await mkdir(config.outputRoot,{recursive:true,mode:0o700});
await mkdir(dir,{recursive:false,mode:0o700});
const report={mp_items:['MP-08','MP-10'],taskId,runId,source:config.source,release:config.release,kernelSha256:config.kernelSha256,provider:'codex',model:'gpt-6.1-sol',effort:'high',seed:42,maxSeconds:config.maxSeconds??600,maxActions:config.maxActions??80,startedAt:new Date().toISOString(),scope:'round3-full',benchmark:'WebMall v1.0',status:'starting',cleanup:[],leaderboard:config.leaderboard};
let sessionId,sliceId,agentId,attachmentId,grader,container,seam='room_create';
let grade;
let interrupted=false;
process.on('SIGTERM',()=>{interrupted=true});
process.on('SIGINT',()=>{interrupted=true});
const checkpoint=()=>writeFile(path.join(dir,'run.json'),JSON.stringify(sanitizeDrillMetadata(report),null,2)+'\n',{mode:0o600});
observeKernelRpcErrors(client,async failure=>{(report.rpcErrors??=[]).push({seam,...failure});await checkpoint()});
const docker=(args,opts={})=>exec('docker',args,{timeout:60000,maxBuffer:1024*1024,...opts});
try {
 const session=one(await client.send(requests.createSessionRequest(config.workspace,config.workspace,'r2next-webmall-task-'+taskId+'-'+runId)),'SessionCreated').session;
 sessionId=session.id;report.sessionId=sessionId;await checkpoint();seam='slice_create';
 let name,slice;
 for(let attempt=0;attempt<3;attempt++){
  name='r2next-webmall-'+runId+'-'+attempt;
  slice=one(await client.send(requests.createSliceRequest({name,displayMode:'headed',displayBackend:'selkies',workspaceMount:config.workspace,base:'clean'})),'SliceCreated').slice;
  sliceId=slice.id;report.sliceId=sliceId;report.kernelId=slice.owner_kernel_id;await checkpoint();
  one(await client.send(requests.bindRoomEnvironmentSliceRequest(sessionId,sliceId)),'RoomEnvironmentSlice');seam='slice_start';
  try{one(await client.send(requests.startSliceRequest(sliceId)),'SliceStarted');break}
  catch(error){if(!/host port.*already in use/.test(error.message)||attempt===2)throw error;await client.send(requests.deleteSliceRequest(sliceId));report.portRetries=attempt+1;}
 }
 container='chariox-slice-'+name;report.container=container;
 const inspected=JSON.parse((await docker(['inspect',container])).stdout)[0];
 assert.equal(inspected.Config.Labels['io.chariox.slice.owner-kernel-id'],slice.owner_kernel_id,'owned slice kernel mismatch');
 assert.equal(inspected.Config.Labels['io.chariox.slice.id'],sliceId,'owned slice ID mismatch');
 assert.equal(inspected.Image,config.sliceImage,'slice image mismatch');
 report.sliceImage=inspected.Image;report.limits={memoryBytes:inspected.HostConfig.Memory,nanoCpus:inspected.HostConfig.NanoCpus};
 report.volumes=inspected.Mounts.filter(m=>m.Type==='volume').map(m=>m.Name);
 assert.equal(inspected.HostConfig.Memory,2048*1024**2);assert.equal(inspected.HostConfig.NanoCpus,1e9);
 one(await client.send(requests.startRoomEnvironmentRequest(sessionId,{css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800})),'RoomEnvironmentUpdated');
 seam='grader_attach';
 await docker(['network','connect',config.siteNetwork,container]);
 await docker(['cp',path.join(import.meta.dirname,'webmall-cdp-forward.mjs'),container+':/tmp/benchwm-cdp-forward.mjs']);
 await docker(['exec','-d','-u','slice',container,'node','/tmp/benchwm-cdp-forward.mjs']);
 const networks=JSON.parse((await docker(['inspect',container])).stdout)[0].NetworkSettings.Networks;
 const endpoint='http://'+networks[config.siteNetwork].IPAddress+':19222';
 for(let i=0;i<40;i++){try{const response=await fetch(endpoint+'/json/version');assert(response.ok);break}catch{if(i===39)throw Error('CDP forward readiness failed');await sleep(250)}}
 grader=spawn(config.python,[path.join(import.meta.dirname,'webmall-grader.py'),config.siteUrls,config.upstream,endpoint,path.join(dir,'grade')],{detached:true,stdio:['pipe','pipe','ignore'],env:{...process.env,PYTHONPYCACHEPREFIX:config.scratch+'/pycache'}});
 report.graderPid=grader.pid;
 const {createInterface}=await import('node:readline');
 let pending;
 createInterface({input:grader.stdout}).on('line',line=>{if(!pending)return;const p=pending;pending=null;clearTimeout(p.timer);try{const v=JSON.parse(line);assert(v.ok,'grader '+v.operation+' '+v.errorClass);p.resolve(v)}catch(e){p.reject(e)}});
 grader.on('exit',()=>{if(pending){clearTimeout(pending.timer);pending.reject(Error('grader exited'));pending=null}});
 grade=request=>new Promise((resolve,reject)=>{assert(!pending);const timer=setTimeout(()=>{pending=null;reject(Error('grader timeout'))},20000);pending={resolve,reject,timer};grader.stdin.write(JSON.stringify(request)+'\n')});
 await grade({op:'setup',task:taskId,goal:task.intent});
 seam='fixture_admission';
 await exec(config.python,[path.join(import.meta.dirname,'webmall-sites.py'),'probe','--upstream',config.upstream,'--lane',config.fixtureLane,'--evidence',config.fixtureEvidence,'--compose-binary',config.composeBinary],{timeout:60000,maxBuffer:65536});
 const admission=JSON.parse(await readFile(path.join(config.fixtureEvidence,'search-admission.json'),'utf8'));
 assert.equal(admission.status,'ready');assert(Date.now()/1000-admission.observedAt<30);
 report.fixtureAdmission=admission;
 seam='agent_spawn';
 const spawned=one(await client.send(requests.spawnAgentRequest(sessionId,'codex','benchwm-codex','gpt-6.1-sol',config.workspace,'high','build','yolo',undefined,undefined,undefined,config.accountProfile)),'AgentSpawned');
 agentId=spawned.agent.id;report.agentId=agentId;
 attachmentId=one(await client.send(requests.attachToSessionRequest(sessionId,'benchwm-runner')),'SessionAttached').attachment.id;
 const prompt=['MP-08 / MP-10: Solve this WebMall task in the actual Room browser. Use ONLY Chariox first-party Browser MCP tools whose names start slice_browser_ or slice_open_url. No shell, scripts, direct HTTP, filesystem, native browser, other MCPs, Computer tools, or grading data. Only interact with the supplied benchmark origins. Observe before acting. Do not create or read local files.',
 'Finish within '+report.maxActions+' total Browser tool calls and '+report.maxSeconds+' seconds. Submit the final result through the official WebMall frontend form, as the task instructs. State failure honestly. After submitting, finish with a short factual response.',
 task.intent].join('\n');
 seam='prompt_submit';
 const api={client,requests,helpers},identity=await submitBenchmarkPrompt(api,{sessionId,attachmentId,agentId,prompt});
 Object.assign(report,identity);await client.send(requests.detachFromSessionRequest(attachmentId));attachmentId=null;report.cleanup.push('prompt_attachment_detached');report.providerStartedAt=new Date().toISOString();report.status='running';await checkpoint();
 seam='provider_settlement';
 const started=Date.now();let completed,actions=[];
 while(Date.now()-started<report.maxSeconds*1000){
  assert(!interrupted,'MP-10 interrupted');
  await guard(); await grade({op:'sample'});
  const t=await getTurn(api,identity);
  const a=one(await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId,null,1000)),'RoomEnvironmentActionHistoryListed').page.actions;
  actions=a.filter(x=>x.actor_id==='agent:'+agentId);report.actions=actions.length;report.providerWallSeconds=(Date.now()-started)/1000;
  if(actions.length>report.maxActions){report.budgetExhausted='actions';break}
  if(t&&['completed','cancelled','failed'].includes(t.lifecycle)){completed=t;break}
  await checkpoint();await sleep(1000);
 }
 if(!completed){report.budgetExhausted??='time';attachmentId=one(await client.send(requests.attachToSessionRequest(sessionId,'benchwm-budget-cancel')),'SessionAttached').attachment.id;await client.send(requests.cancelActivePromptRequest(sessionId,attachmentId,agentId));throw Error('provider budget exhausted')}
 report.turnId=completed.turn_id;report.lifecycle=completed.lifecycle;
 report.promotedPromptId=identity.promotedPromptId??null;
 const entries=assembleTurnEntries(await loadTurnHistory({client,requests},{sessionId,agentId,turn:completed}),helpers);
 report.providerUnauthorized=entries.some(x=>x.entry.kind==='provider_error'&&/\b401\b|unauthorized/i.test(x.entry.text??''));
 if(report.providerUnauthorized)console.log('MP-08 / MP-10 coordinator notice: linked provider returned 401/unauthorized');
 const tools=entries.filter(x=>x.entry.kind==='provider_tool');
 report.providerToolEntries=tools.length;
 // Tool transcripts include their upstream harness tool identity. Preserve a
 // private audit, then require no native non-Browser tools in this task.
 const toolUpdates=tools.map(x=>{try{const u=JSON.parse(x.entry.text);return {id:u.id,tool:u.tool,status:u.status,input:u.input}}catch{return {unparseable:true}}});
 const byId=new Map();for(const u of toolUpdates)byId.set(u.id??randomUUID(),u);
 const forbidden=toolUpdates.filter(u=>u.unparseable||! /^(slice_browser_[a-z_]+|slice_open_url)$/.test(roomProviderToolName(u.tool)));
 report.providerToolCalls=byId.size;report.toolNames=[...new Set(toolUpdates.map(u=>u.tool))];
 report.forbiddenToolCount=forbidden.length;
 await writeFile(path.join(dir,'tool-audit.json'),JSON.stringify(toolUpdates),{mode:0o600});
 assert.equal(forbidden.length,0,'task used a forbidden or unparseable provider tool');
 assert(report.providerToolCalls<=report.maxActions,'provider tool-call budget exceeded');
 assert(actions.length>0&&actions.every(x=>x.mode==='browser'),'task lacks exclusive attributed Browser actions');
 seam='official_scoring';
 report.lifecycle=completed.lifecycle;
 report.usageTokensTotal=null;
 try { const state=one(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState');const id=state.agent_activity?.[agentId]?.last_completed_turn?.provider_run_id;if(id)report.usageTokensTotal=one(await client.send(requests.getProviderRunRequest(id)),'ProviderRun').provider_run.usage_tokens_total; } catch {}
 assert.equal(completed.lifecycle,'completed','provider turn did not complete');
 assert(!entries.some(x=>x.entry.kind==='provider_error'),'provider failed');
 report.grade=await grade({op:'finish'});
 await writeFile(path.join(dir,'agent-response.json'),JSON.stringify(entries.filter(x=>x.entry.kind==='provider_output').map(x=>x.entry.text)),{mode:0o600});
 report.status='scored';
 await writeFile(path.join(dir,'actions.json'),JSON.stringify(actions,null,2),{mode:0o600});
} catch(error){report.status='RED';report.firstFailingSeam=seam;report.failureCode=error.code??error.name;report.failure=rpcErrorRecord(error)}
finally{
 if(agentId&&sessionId){try{const state=one(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState');if(state.session.agents.find(a=>a.id===agentId)?.is_processing){attachmentId??=one(await client.send(requests.attachToSessionRequest(sessionId,'benchwm-cleanup')),'SessionAttached').attachment.id;await client.send(requests.cancelActivePromptRequest(sessionId,attachmentId,agentId));for(let i=0;i<40;i++){const s=one(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState');if(!s.session.agents.find(a=>a.id===agentId)?.is_processing)break;await sleep(250)}}}catch{report.cleanup.push('provider_cancel_failed')}}
 if(grader){if(!report.grade)try{report.grade=await grade({op:'finish'})}catch{report.gradingIncomplete=true};grader.stdin.end();try{assert(await stopOwnedProcess(grader,{detached:true}),'grader exit acknowledgement missing');report.cleanup.push('grader_stopped')}catch{report.cleanup.push('grader_stop_failed')}}
 if(attachmentId)try{await client.send(requests.detachFromSessionRequest(attachmentId));report.cleanup.push('attachment_detached')}catch{report.cleanup.push('attachment_detach_failed')}
 if(sessionId)try{await client.send(requests.deleteSessionRequest(sessionId,config.workspace));report.cleanup.push('owned_room_deleted')}catch{report.cleanup.push('room_delete_failed')}
 if(sliceId)try{await client.send(requests.stopSliceRequest(sliceId));await client.send(requests.deleteSliceRequest(sliceId));report.cleanup.push('owned_slice_deleted')}catch{report.cleanup.push('slice_delete_failed')}
 if(container){report.containerGone=await docker(['inspect',container]).then(()=>false,()=>true);if(!report.containerGone)report.cleanup.push('container_cleanup_failed');}
 report.volumesGone=true;
 for(const volume of report.volumes??[]){const exists=await docker(['volume','inspect',volume]).then(()=>true,()=>false);if(exists)report.volumesGone=false;}
 if(!report.volumesGone)report.cleanup.push('volume_cleanup_failed');
 if(report.cleanup.some(x=>x.endsWith('_failed'))){report.status='RED';report.firstFailingSeam??='cleanup'}
 report.finishedAt=new Date().toISOString();await checkpoint();await client.close();clearInterval(keepAlive);
 console.log(JSON.stringify({mp_items:report.mp_items,taskId,status:report.status,seam:report.firstFailingSeam,actions:report.actions,providerWallSeconds:report.providerWallSeconds,completion:report.grade?.officialMetrics?.avg_task_completion_rate,cleanup:report.cleanup}));
}
process.exitCode=report.status==='scored'?0:1;
