// MP-08 / MP-10 / MP-11: ordinary second-agent permission during source compaction.
import {sleep} from './runtime.mjs';
import {access} from 'node:fs/promises';
export async function prepareCompactionPeer(run){
 const sourceId=(await run.tui.send('snapshot')).session.focusedAgentId,alias='compaction-peer-'+Date.now(),marker='CX04_PEER_'+Date.now(),note=run.root+'/audit-notes/peer-'+Date.now()+'.txt';
 await run.command('agent spawn '+alias+' claude-p/claude-haiku-5-5');
 let peer;for(let n=0;n<120;n++){peer=(await run.tui.send('snapshot')).session.agents.find(a=>a.alias===alias);if(peer)break;await sleep(250)}if(!peer)throw Error('Ordinary second agent was not created');
 await run.command('agent focus '+peer.id);await run.command('agent account '+peer.id+' '+run.profiles.claude);await run.command('agent permissions '+peer.id+' required');await run.command('agent mode '+peer.id+' build');
 await run.tui.send('submit_prompt',{prompt:'Independently audit apps/kernel/src/provider/launch_contract.rs for the same-account model-change invariant. Read the actual source, then use a shell command to create a short audit note at '+note+' with one verified function name and its invariant. Print the filename and read back the note, then delete that exact note. This scratch path is outside the checkout; never modify source or read credentials or other runtime state. Request the normal permission. End with '+marker+'.'});
 let interaction;for(let n=0;n<240;n++){const s=await run.tui.send('snapshot');if(s.session.agents.find(a=>a.id===peer.id)?.state==='Error'){for(const row of s.agentPanes[peer.id]??[])if(row.role==='error'&&row.blobCollapsed)await run.tui.send('toggle_blob',{agentId:peer.id,entryId:row.id,collapsed:false});await sleep(1000);const expanded=await run.capture('peer-provider-error');throw Error('Second-agent provider failed: '+(expanded.agentPanes[peer.id]??[]).filter(x=>x.role==='error').map(x=>x.text??'').join(' ').slice(0,800));}interaction=(s.interactions??[]).find(i=>i.agentId===peer.id&&/permission|approval|approve|allow/i.test(i.title??''));if(interaction)break;await sleep(250)}if(!interaction)throw Error('Real second-agent permission was not offered');
 await run.capture('peer-permission-pending');
 return {sourceId,peerId:peer.id,interactionId:interaction.id,marker,note};
}
export async function approveCompactionPeer(run,peer,isPending,focus){
 if(!isPending())throw Error('Source operation ended before unrelated permission response');
 const started=Date.now(),responses=[];await focus(peer.peerId);
 async function reply(id){const s=await run.tui.send('snapshot');if(!s.interactions.some(i=>i.id===id&&i.agentId===peer.peerId))throw Error('Second-agent permission lost before response');await run.tui.send('interaction_submit',{choiceIndex:0});responses.push({interactionId:id,atMs:Date.now(),sourcePending:isPending()});}
 try{
  if(!isPending())throw Error('Source operation ended before peer reply');await reply(peer.interactionId);let doneSince;
  for(let n=0;n<480;n++){
   const s=await run.tui.send('snapshot'),agent=s.session.agents.find(a=>a.id===peer.peerId),rows=s.agentPanes[peer.peerId]??[];
   const pending=(s.interactions??[]).find(i=>i.agentId===peer.peerId&&i.kind==='permission');if(pending){await focus(peer.peerId);await reply(pending.id);continue;}
   for(const row of rows)if(row.blobCollapsed&&row.historyBlobLoaded===false)await run.tui.send('toggle_blob',{agentId:peer.peerId,entryId:row.id,collapsed:false});
   if(agent?.state==='Error'){await run.capture('peer-provider-error-after-reply');throw Error('Second-agent audit entered Error; inspect expanded peer pane');}
   const toolProof=rows.some(x=>x.role==='tool'&&/COMPLETED/.test(x.text??'')&&(x.text?.split('**Output**')[1]??'').includes(peer.note))&&rows.some(x=>x.role==='tool'&&x.text?.includes('launch_contract'));
   if(!agent?.isProcessing&&agent?.state!=='Working'&&toolProof&&rows.some(x=>x.role==='assistant'&&x.text?.trim())){
    doneSince??=Date.now();if(Date.now()-doneSince<5000){await sleep(250);continue;}
    await access(peer.note).then(()=>{throw Error('Second-agent scratch note remains')},e=>{if(e.code!=='ENOENT')throw e});
    const result={peerId:peer.peerId,startedAtMs:started,replyRecordedAtMs:responses[0].atMs,replyElapsedMs:responses[0].atMs-started,completedMs:Date.now()-started,sourceOperationPendingAtReply:responses[0].sourcePending,responses,markerObserved:rows.some(x=>x.role==='assistant'&&x.text?.includes(peer.marker)),realToolResult:true,realRepositoryAudit:true,scratchRemoved:true,limits:'Tool completion/readback/removal proved; no full UI acceptance or invented marker claimed'};await run.record('peer-permission-completed',result);return result;
   }
   await sleep(250);
  }
  throw Error('Second agent did not complete its audited tool work after permission response');
 }finally{await focus(peer.sourceId);}
}
