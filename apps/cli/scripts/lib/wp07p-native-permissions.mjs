// MP-08/MP-10: one real official-provider RuntimeInteraction, Web/local/remote TUI.
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import path from 'node:path';
const mp_items=['MP-08','MP-10'];
const unwrap=(r,k)=>{assert.ok(r?.[k],`MP-08/MP-10 ${k} absent`);return r[k]};
export async function runPermissions(i){
 const {client,requests,sessionId,provider,options,fixtureWorkspace,publicEvidenceRoot,waitFor,withTimeout,localAutomation,remoteAutomation,web}=i;
 const report={mp_items,provider:options.provider,model:options.model,agentId:provider.agentId,status:'RED',interaction:null,web:'pending',localTui:false,remoteTui:false};let attachment,heartbeat;
 try{
  if(web){
   try{
    const baseline=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[provider.agentId],4)),'SessionHistoryOutline');const seen=new Set(baseline.agents?.find(a=>a.agent_id===provider.agentId)?.turns?.map(t=>t.prompt_id)??[]);
    await web.observe('web-prompt');
    const t=await waitFor(async()=>{const h=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[provider.agentId],4)),'SessionHistoryOutline');return h.agents?.find(a=>a.agent_id===provider.agentId)?.turns?.find(t=>!seen.has(t.prompt_id)&&t.lifecycle==='completed')},90000,'MP-08/MP-10 Web-origin prompt did not settle');
    report.webPrompt={status:'PASS',promptId:t.prompt_id,origin:'production terminal composer'};
   }catch(error){report.webPrompt={status:'RED',reason:error.message.split('; last snapshot')[0]}}
  }
  await client.send(requests.updateAgentConfigRequest({sessionId,agentId:provider.agentId,permissionLevel:'required'}));
  await client.send(requests.focusAgentRequest(sessionId,provider.agentId));
  attachment=unwrap(await client.send(requests.attachToSessionRequest(sessionId,'wp07-native-permission')),'SessionAttached').attachment;
  heartbeat=setInterval(()=>client.send(requests.pollRuntimeNoticesRequest(sessionId,attachment.id)).catch(()=>{report.heartbeatFailures=(report.heartbeatFailures??0)+1}),10000);
  const file=path.join(fixtureWorkspace,'wp07-native-approval.txt');
  // Writing to the authorized disposable workspace remains low impact. Codex
  // required permissions need an explicit escalation request outside its sandbox.
  const prompt=options.provider==='codex'?`MP-08/MP-10 native permission drill: run a shell command with sandbox_permissions=require_escalated and a concise justification to print WP07_NATIVE_APPROVAL_OK; do not write outside the workspace or contact a service. Ask the native approval, await its normal reply, then report exactly WP07_NATIVE_APPROVAL_OK.`:`MP-08/MP-10 native permission drill: call your native bash tool with command=python3 -c "print('WP07_NATIVE_APPROVAL_OK')". Wait for its native permission approval, then report exactly WP07_NATIVE_APPROVAL_OK. No files, networks or account changes.`;
  const submitted=unwrap(await client.send(requests.submitPromptRequest(sessionId,attachment.id,provider.agentId,prompt,[])),'PromptSubmitted');const promptId=(submitted.outcome.Started??submitted.outcome.Queued)?.prompt?.id;assert.ok(promptId);report.promptId=promptId;
  const interaction=await waitFor(async()=>{await client.send(requests.pollRuntimeNoticesRequest(sessionId,attachment.id));const s=unwrap(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState').session;const own=(s.active_interactions??[]).filter(x=>x.agent_id===provider.agentId);assert.ok(own.length<=1,'MP-08/MP-10 duplicate native authority');if(own.length)return own[0];const h=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[provider.agentId],2)),'SessionHistoryOutline');if(h.agents?.find(a=>a.agent_id===provider.agentId)?.turns?.some(t=>t.prompt_id===promptId&&t.lifecycle==='completed'))throw new Error('MP-08/MP-10 provider completed without native permission');return false},90000,'MP-08/MP-10 native permission absent');
  report.interaction={id:interaction.id,kind:interaction.kind,level:interaction.level,choices:interaction.choices.map(c=>c.id)};
  for(const [key,tui] of [['localTui',localAutomation],['remoteTui',remoteAutomation]]){
   await waitFor(async()=>{const s=await tui.send('snapshot');return s.interactions?.some(x=>x.id===interaction.id&&x.agentId===provider.agentId)},15000,'MP-08/MP-10 TUI interaction missing');report[key]=true;
  }
  if(web){try{await web.observe('permission-'+interaction.id);report.web='observed';await web.observe('approve-'+interaction.id);report.replyOrigin='Web product choice'}catch(error){report.web='RED';report.webError=error.message;await client.send(requests.respondToInteractionRequest(sessionId,interaction.id,'allow_once'));report.replyOrigin='kernel client after Web failure'}}
  else {await client.send(requests.respondToInteractionRequest(sessionId,interaction.id,'allow_once'));report.replyOrigin='kernel client; Web pending'}
  await waitFor(async()=>{await client.send(requests.pollRuntimeNoticesRequest(sessionId,attachment.id));const s=unwrap(await client.send(requests.getSessionStateRequest(sessionId)),'SessionState').session;const h=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[provider.agentId],2)),'SessionHistoryOutline');return !s.active_interactions?.some(x=>x.id===interaction.id)&&!s.agents.find(a=>a.id===provider.agentId)?.is_processing&&h.agents?.find(a=>a.agent_id===provider.agentId)?.turns?.some(t=>t.prompt_id===promptId&&t.lifecycle==='completed')},90000,'MP-08/MP-10 permission settlement');
  report.status='GREEN';report.singleKernelAuthority=true;
 }catch(error){report.error=error.message.split('; last snapshot')[0]}
 finally{
  if(heartbeat)clearInterval(heartbeat);
  try{
   const h=unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId,[provider.agentId],4)),'SessionHistoryOutline');
   const t=h.agents?.find(a=>a.agent_id===provider.agentId)?.turns?.find(t=>t.prompt_id===report.promptId);
   report.diagnostic={lifecycle:t?.lifecycle,tools:[],markerInAssistant:false};const entries=[...(t?.entries??[]),...(t?.summary?[t.summary]:[])];
   for(const b of t?.blobs??[]){if(b.total_chars>250000)continue;const d=unwrap(await client.send(requests.getSessionHistoryBlobContentRequest(sessionId,provider.agentId,b.blob_id)),'SessionHistoryBlobContent');entries.push(...(d.entries??[]))}
   for(const i of entries){const e=i.entry;if(!e?.text)continue;if(e.kind==='provider_output')report.diagnostic.markerInAssistant ||= e.text.includes('WP07_NATIVE_APPROVAL_OK');if(e.kind!=='provider_tool')continue;let x;try{x=JSON.parse(e.text)}catch{continue}report.diagnostic.tools.push({tool:String(x.tool??'').slice(0,100),status:x.status,outputMarker:String(x.output??'').includes('WP07_NATIVE_APPROVAL_OK')})}
  }catch{report.diagnostic={unavailable:true}}
  if(attachment)await client.send(requests.detachFromSessionRequest(attachment.id)).catch(()=>{});await client.send(requests.updateAgentConfigRequest({sessionId,agentId:provider.agentId,permissionLevel:'yolo'})).catch(()=>{});await writeFile(path.join(publicEvidenceRoot,'native-permission.json'),JSON.stringify(report,null,2)+'\n')}
 return report;
}
