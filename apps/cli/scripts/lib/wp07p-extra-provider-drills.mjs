import assert from 'node:assert/strict';
import http from 'node:http';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {captureRoomProviderDiagnostic} from './live-room-provider-diagnostic.mjs';
import {spawn} from 'node:child_process';
import {writeFile,mkdir,readFile} from 'node:fs/promises';
const mp_items=['MP-08','MP-10'];
const laneRoot=process.env.WP07_D_LANE_ROOT??'/root/.chariox/dev/browser-resume-20260930/agents/wp07p';
const unwrap=(r,v)=>{assert.ok(r?.[v],`missing ${v}`);return r[v]};
export async function roomsExtra(input){
 const {client,requests,sessionId,options,provider,publicEvidenceRoot,screenshot,waitFor,withTimeout,docker,containerName}=input;
 const out=path.join(publicEvidenceRoot,'extra');await mkdir(out,{mode:0o700});
 const baseline=Math.max(0,...unwrap(await client.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId,null,100)),'RoomEnvironmentActionHistoryListed').page.actions.map(a=>a.sequence));
 const counts=new Map();const submissions=[];
 const server=http.createServer(async(req,res)=>{
  const route=new URL(req.url,'http://local').pathname;
  counts.set(route,(counts.get(route)??0)+1);
  if(route==='/e-a'||route==='/e-b'){
   if(counts.get(route)>1)await new Promise(r=>setTimeout(r,3800));
   res.setHeader('Content-Type','text/html');res.end(`<title>Room ${route}</title><h1>Drill E probe</h1>`+Array.from({length:3000},(_,i)=>`<button>Drill E probe ${i}</button>`).join(''));return;
  }
  if(route==='/vendors'){res.end('<title>Room vendor research</title><h1>Office printers</h1><table><tr><th>Vendor</th><th>Price USD</th><th>Warranty years</th></tr><tr><td>Acme</td><td>200</td><td>2</td></tr><tr><td>Beta</td><td>160</td><td>1</td></tr><tr><td>Gamma</td><td>250</td><td>3</td></tr></table><a href="/crm">Submit procurement request</a>');return}
  if(route==='/crm'&&req.method==='GET'){res.end('<title>Room CRM</title><form method="POST"><label>Vendor<input name="vendor"></label><label>Reason<input name="reason"></label><button>Send request</button></form>');return}
  if(route==='/crm'&&req.method==='POST'){let body='';for await(const chunk of req)body+=chunk;submissions.push(Object.fromEntries(new URLSearchParams(body)));res.end('<title>Room CRM saved</title><h1>REQUEST_ACCEPTED</h1>');return}
  res.writeHead(404);res.end('Unknown rooms fixture');
 });
 await new Promise(resolve=>server.listen(0,'0.0.0.0',resolve));
 const origin=`http://host.docker.internal:${server.address().port}`;
 const runCli=async(name,args)=>{const child=spawn('node',[path.join(laneRoot, name==='live-browser-computer-drill-e-scenario.mjs'?'drill-e-live.mjs':'office-suite-live.mjs'),...args],{stdio:['ignore','pipe','pipe']});let output='';child.stdout.on('data',x=>output+=x);child.stderr.on('data',()=>{});const code=await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',resolve)});return {mp_items,helperCommit:process.env.ROOMS_HELPER_HEAD??'unknown',command:['node',path.join(laneRoot,name==='live-browser-computer-drill-e-scenario.mjs'?'drill-e-live.mjs':'office-suite-live.mjs'),...args],exit_code:code,summary:output.trim()}};
 const turn=async(agentId,prompt)=>{
  const attachment=unwrap(await client.send(requests.attachToSessionRequest(sessionId,`rooms-extra-setup-${agentId}-${randomUUID()}`)),'SessionAttached').attachment;
  let id;
  try{
   const submitted=unwrap(await client.send(requests.submitPromptRequest(sessionId,attachment.id,agentId,prompt,[])),'PromptSubmitted');id=(submitted.outcome.Started??submitted.outcome.Queued)?.prompt?.id;assert.ok(id);
   await waitFor(async()=>{await client.send(requests.pollRuntimeNoticesRequest(sessionId,attachment.id));const h=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[agentId],2)),'SessionHistoryOutline');const state=unwrap(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState');const a=state.agent_activity?.[agentId];if(a?.status==='error'&&a?.prompt_status==='none')throw new Error('MP-08/MP-10 official provider entered error state during Room setup/work');return a?.status==='idle'&&a?.prompt_status==='none'&&h.agents?.find(a=>a.agent_id===agentId)?.turns?.some(t=>t.prompt_id===id&&t.lifecycle==='completed')},180000,'rooms setup turn timeout');
   return id;
  }finally{await client.send(requests.detachFromSessionRequest(attachment.id))}
 };

 try{
  const otherAgents=[];
  if(!process.env.ROOMS_SCENARIO_ONLY){
  const alternate=options.provider==='codex'?'opencode':'codex';
  const alternateRoot='/root/.chariox/dev/provider-runtimes/browser-computer/acct-686/home/state/provider-accounts/local/'+(alternate==='codex'?'codex/codex-1-njimhtya/codex':'opencode/opencode-1-76jonxf5');
  const profile=unwrap(await client.send(requests.linkProviderAccountProfileRequest(alternate,'wp07p-E-alternate',alternateRoot)),'ProviderAccountProfile').profile;
  const auth=unwrap(await client.send(requests.getProviderAuthStatusRequest(alternate,profile.profile_id)),'ProviderAuthStatus').status;assert.equal(auth.auth_state,'authenticated');
  const actors=[{alias:'drill-reader-b',provider:options.provider,model:options.model,account:options.accountProfile},{alias:'drill-worker-c',provider:alternate,model:alternate==='codex'?'gpt-6.1-sol':'opencode-go/gpt-6-luna',account:profile.profile_id}];
  for(const actor of actors){const a=unwrap(await client.send(requests.spawnAgentRequest(sessionId,actor.provider,actor.alias,actor.model,input.fixtureWorkspace,'low','build','yolo',undefined,undefined,undefined,actor.account)),'AgentSpawned').agent;otherAgents.push(a.id)}
  await writeFile(path.join(out,'drill-e-provider-actors.json'),JSON.stringify({mp_items,actors:[{agentId:provider.agentId,provider:options.provider},...actors.map((a,i)=>({agentId:otherAgents[i],provider:a.provider}))],scope:'mixed official provider realism smoke; deterministic timing belongs to ctlconc'})+'\n');
  await docker(['exec','-u','slice',containerName,'node','--input-type=module','-e',`for(const url of ${JSON.stringify([origin+'/e-a',origin+'/e-b'])}){const r=await fetch('http://127.0.0.1:9222/json/new?'+encodeURIComponent(url),{method:'PUT'});if(!r.ok)throw new Error('tab setup failed');}`]);
  const setup=await Promise.allSettled([provider.agentId,...otherAgents].map(agentId=>turn(agentId,'MP-08/MP-10 Room setup: call only slice_browser_status once to discover the current Room tab inventory, then stop.')));
  const setupFailure=setup.find(result=>result.status==='rejected');if(setupFailure)throw setupFailure.reason;const setupPromptIds=setup.map(result=>result.value);
  let env=unwrap(await client.send(requests.getRoomEnvironmentStateRequest(sessionId)),'RoomEnvironmentState').environment;
  const same=env.tabs.find(t=>t.url===origin+'/e-a'),other=env.tabs.find(t=>t.url===origin+'/e-b');assert.ok(same&&other,'Room did not discover setup tabs');
  await client.send(requests.requestRoomEnvironmentInputTakeoverRequest(sessionId,{kind:"browser_tab",id:same.tab_id}));
  await client.send(requests.submitRoomEnvironmentBrowserActionRequest(sessionId,env.runtime_generation,randomUUID(),{kind:'tab',action:'activate',tab_id:same.tab_id}));
  await client.send(requests.releaseRoomEnvironmentInputRequest(sessionId,{kind:'browser_tab',id:same.tab_id}));
  await screenshot('drill-e-before');
  const e=await runCli('live-browser-computer-drill-e-scenario.mjs',['--execute','--kernel-url',input.kernelUrl,'--session',sessionId,'--agent-a',provider.agentId,'--agent-b',otherAgents[0],'--agent-c',otherAgents[1],'--same-tab',same.tab_id,'--other-tab',other.tab_id,'--timeout-ms','180000','--output',path.join(out,'drill-e.json')]);
  await writeFile(path.join(out,'drill-e-command.json'),JSON.stringify({...e,setupPromptIds,fixture:{reloadDelayMs:3800,buttons:3000,counts:Object.fromEntries(counts)}} ,null,2)+'\n',{mode:0o600});
  await screenshot('drill-e-after');
  await (await import('./wp07p-drill-e-realism.mjs')).runERealism({...input,agentIds:[provider.agentId,...otherAgents],sameTabId:same.tab_id,otherTabId:other.tab_id,turn,waitFor,out});
  for(const agentId of [provider.agentId,...otherAgents]){const diagnostic=await captureRoomProviderDiagnostic({...input,agentId});await writeFile(path.join(out,`e-provider-${agentId}.json`),JSON.stringify({mp_items,diagnostic},null,2)+'\n',{mode:0o600})}
  }
  if(process.env.ROOMS_E_ONLY)return;
  // The suite captures observations; independent CRM/result checks remain required.
  const scenarios=process.env.ROOMS_SCENARIO_ONLY?[process.env.ROOMS_SCENARIO_ONLY]:['vendor-research-crm','public-api-extension'];
  const tasks={
   'vendor-research-crm':`MP-08/MP-10: Research the three office printer vendors at ${origin}/vendors using Chariox Browser tools. Create a small Python script extension named rooms_vendor using the Chariox runtime registration and explicit grant path (register_environment name rooms_python, runtime {type: python, python: /usr/bin/python3}; script defines run and test_run with docstrings and primitive typed parameters or no parameters; register_script_path with environment rooms_python; request_extension kind script, name rooms_vendor, environment rooms_python). Discover the registered MCP tool and use it to normalize those three rows. Run that extension. Save a JSON comparison artifact to ${input.fixtureWorkspace}/vendor_comparison.json in your workspace, choose a printer with warranty at least two years and the lowest cost, then fill Vendor and Reason at ${origin}/crm and submit exactly once through Chariox Browser tools. This is an owned local fixture, no credentials or external accounts. Take a Chariox screenshot of the accepted request. Provider shell/file tools may create the extension and result artifact only; do not make direct HTTP calls to the form.`,
   'public-api-extension':`MP-08/MP-10: Create and register a Python script extension named rooms_public_api using Chariox runtime tools, then explicitly grant it and execute it in this same session. The extension must GET the public no-auth API https://api.github.com/repos/microsoft/vscode with a User-Agent header, read the public repository name and open_issues_count, and return a bounded JSON summary. Save that actual tool result as JSON to ${input.fixtureWorkspace}/rooms_public_api_summary.json in your workspace for an office support capacity check. A final chat reply is not a saved artifact. Open the repository's public GitHub page with Chariox Browser tools and capture a Chariox screenshot.  Use the documented Python extension contract: register_environment config has name rooms_python and runtime {type: "python", python: "/usr/bin/python3"}; source defines callable run and test_run, each with a docstring, primitive typed parameters (str or int, or no parameters) and returns a JSON-compatible result. Register the path with name rooms_public_api, environment rooms_python, then request_extension kind script, name rooms_public_api, environment rooms_python. The registered script becomes an MCP tool named rooms_public_api; search/discover it if needed. A local Python execution outside Chariox does not satisfy running the registered tool. If the provider cannot refresh its tool catalog after grant, stop and report that exact limitation. Never access provider state, credentials, or another service; this authorizes read-only public HTTP plus this task's local extension/artifact writes.`,
  };
  const args=['--execute','--kernel-url',input.kernelUrl,'--session',sessionId,'--agent',provider.agentId,'--output-dir',path.join(out,'office-suite'),'--timeout-ms','300000'];
  for(const scenario of scenarios)args.push('--scenario',scenario,'--task',scenario+'='+tasks[scenario],'--confirm-external-actions',scenario);
  const suite=await runCli('live-room-office-scenario-suite-drill.mjs',args);
  // MP-08/MP-10: the suite captures its original prompt; also wait for the kernel's automatic resume continuation.
  const wanted=scenarios.includes('public-api-extension')?'rooms_public_api':'rooms_vendor';
  let refreshWait={mp_items,status:'pending',tool:wanted,verificationPromptRequired:false};

  try{
   await waitFor(async()=>{
    await (await import(path.join(laneRoot,'extension-evidence.mjs'))).captureExtensionEvidence({...input,out});
    const e=JSON.parse(await readFile(path.join(out,'extension-evidence.json'),'utf8'));
    if(e.tools.some(t=>t.tool===wanted&&t.status==='completed'&&t.businessResult))return true;
    const state=unwrap(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState');
    const activity=state.agent_activity?.[provider.agentId];
    return false;
   },300000,'MP-08/MP-10 resumed extension invocation missing');
   await waitFor(async()=>{const state=unwrap(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState');const activity=state.agent_activity?.[provider.agentId];return activity?.status==='idle'&&activity?.prompt_status==='none'},300000,'MP-08/MP-10 resumed provider did not settle');
   refreshWait.status='passed';
  }catch(error){refreshWait={...refreshWait,status:'failed',reason:error.message.split('; last snapshot')[0]}}
  await writeFile(path.join(out,'catalog-refresh-wait.json'),JSON.stringify(refreshWait)+'\n',{mode:0o600});
  await (await import(path.join(laneRoot,'extension-evidence.mjs'))).captureExtensionEvidence({...input,out});
  await writeFile(path.join(out,'suite-provider-diagnostic.json'),JSON.stringify({mp_items,diagnostic:await captureRoomProviderDiagnostic({...input,agentId:provider.agentId})},null,2)+'\n',{mode:0o600});
  await writeFile(path.join(out,'office-suite-command.json'),JSON.stringify(suite,null,2)+'\n',{mode:0o600});
  await screenshot('office-suite-final').catch(()=>undefined);
  await writeFile(path.join(out,'fixture-verification.json'),JSON.stringify({mp_items,submissions,crmExactOnce:submissions.length===1,selectedVendor:submissions[0]?.vendor,expectedVendor:'Acme',excludedScenarios:{'email-gated-onboarding':'No authorized public service, email account or vault handle was supplied','support-ticket-lifecycle':'No authorized helpdesk/email sandbox account or vault handle was supplied','document-intake-follow-up':'Covered separately by real graphical document/edit/upload/mail Drill D; does not establish public email service acceptance'}},null,2)+'\n',{mode:0o600});
  await (await import('./wp07p-extra-client-proof.mjs')).proveExtraClients(input,baseline,out);
 }catch(error){await writeFile(path.join(out,'extra-failure.json'),JSON.stringify({mp_items,name:error.name,message:error.message.split('; last snapshot')[0]})+'\n',{mode:0o600});}
 finally{server.closeAllConnections();await new Promise(r=>server.close(r));}
}
