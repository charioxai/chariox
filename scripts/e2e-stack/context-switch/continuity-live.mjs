// MP-08 / MP-10 / MP-11: two-hour official-provider leased-worker continuity through real clients.
import {r,init,finish,select,workload,probe,enrollClaude} from './common.mjs';
import {sleep} from './runtime.mjs';
import {webClients} from './web-live.mjs';
import {scoreContextValidationRecall} from './context-switch-validation-records.mjs';
import {performance} from 'node:perf_hooks';
const result={MP:['MP-08','MP-10','MP-11'],status:'RUNNING',scope:process.env.CTXVAL_CONTINUITY_SCOPE??'requested2h remote continuity, not the complete3h CX-02 matrix',started:new Date().toISOString(),cells:[],latency:[]};
const openCodeModel=process.env.CTXVAL_OPENCODE_MODEL??'opencode-go/deepseek-v4.1-flash';
let worker,remote,web=[],facts,start,switched=0;
async function cell(name,fn){const row={cell:name,status:'FAIL',started:new Date().toISOString()};try{Object.assign(row,await fn());row.status='PASS'}catch(e){row.firstSeam=String(e?.message??e);result.cells.push(row);await r.record('result',result);throw e}row.finished=new Date().toISOString();result.cells.push(row);await r.record('result',result);return row;}
async function visibleProbe(client){
 const marker='CTXVAL_CONTINUITY_'+client.dpr+'_'+Date.now(),input=client.page.locator('[data-terminal-prompt]:not(:disabled)');
 const prompt='Without tools, recall the nine existing audit records as JSON with exactly '+Object.keys(facts).join(', ')+'. Preserve their exact values. python_result must contain exactly 64 lowercase hexadecimal characters; copy all 64 characters, including the final character, from the conversation. Do not use tools. End with '+marker+'.';
 await input.fill(prompt);
 await client.page.evaluate(marker=>{window.__ctxlatency={marker,start:null,visible:null};document.addEventListener('keydown',function handler(e){if(e.key!=='Enter'||!e.target.matches('[data-terminal-prompt]'))return;document.removeEventListener('keydown',handler,true);window.__ctxlatency.start=performance.now();const observer=new MutationObserver(()=>{const walker=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT);let node;while(node=walker.nextNode()){if(node.nodeValue.includes(marker)&&node.parentElement?.getClientRects().length&&!node.parentElement.closest('input,textarea,[contenteditable=true],[data-terminal-prompt]')){window.__ctxlatency.visible=performance.now();observer.disconnect();break;}}});observer.observe(document.body,{subtree:true,childList:true,characterData:true});},true)},marker);
 await input.press('Enter');
 await client.page.waitForFunction(()=>window.__ctxlatency?.visible!=null,null,{timeout:30000});
 const latency=await client.page.evaluate(()=>window.__ctxlatency.visible-window.__ctxlatency.start);
 result.latency.push({dpr:client.dpr,latencyMs:latency,kind:'Enter to visible prompt echo'});
 const turn=await r.turn('',marker,900000,false),score=scoreContextValidationRecall(turn.state.agentPanes[turn.state.session.focusedAgentId],marker,facts);
 await r.record('continuity-score-'+Date.now(),{score,latencyMs:latency,dpr:client.dpr});await client.capture('continuity-probe');
 if(score.score!==9||score.toolCount)throw Error('No-tool recall '+score.score+'/9');
 return {score:score.score,latencyMs:latency,dpr:client.dpr};
}
async function continuity(){const s=await r.tui.send('snapshot');if(s.daemonDisconnected)throw Error('Remote TUI disconnected');const a=s.session.agents.find(a=>a.id===s.session.focusedAgentId);if(!JSON.stringify(a.remoteExecution).includes(worker.enrollment.daemon_id))throw Error('Agent lost leased-worker placement');for(const c of web){if(!await c.page.locator('[data-terminal-prompt]:not(:disabled)').count())throw Error('Web composer lost connectivity');if(!a.isProcessing&&a.state!=='Working'){await c.page.locator(`.freeform-agent-pane[data-agent-id="${a.id}"] .freeform-status-badge`).filter({hasText:'IDLE'}).waitFor({timeout:10000});}}
return {agentId:a.id,remoteExecution:a.remoteExecution};}
try{
 ({worker,remote}=await init({leased:true}));await enrollClaude();
 facts=await workload(process.env.CTXVAL_BASELINE_PROVIDER??'codex',process.env.CTXVAL_BASELINE_MODEL??'gpt-6-luna',3);
 if(process.env.CTXVAL_INITIAL_PROVIDER){await select(process.env.CTXVAL_INITIAL_PROVIDER,process.env.CTXVAL_INITIAL_MODEL??'claude-haiku-5-5');await probe(facts);}web=await webClients(r,remote);start=performance.now();
 await cell('ordinary-queue-continuity',async()=>{
  const id=(await r.tui.send('snapshot')).session.focusedAgentId,first='CTXVAL_QUEUE_FIRST_'+Date.now(),second='CTXVAL_QUEUE_SECOND_'+Date.now();
  await r.tui.send('submit_prompt',{prompt:'Continue the actual read-only audit. Read structured_prompt_dispatch.rs and its brief_dispatch_review.rs regressions, compare the admission sequence with one test and cite exact source lines. Preserve the nine audit records; no edits, builds, delegation, subagents or credential/runtime reads. End with '+first+'.'});
  let working=false;for(let n=0;n<80;n++){const a=(await r.tui.send('snapshot')).session.agents.find(a=>a.id===id);if(a.isProcessing||a.state==='Working'){working=true;break}await sleep(100)}if(!working)throw Error('Queue source turn never became active');
  await r.tui.send('submit_prompt',{prompt:'Without tools, recall the accepted decision from our nine audit records. Return JSON with exactly one field named decision, copying the stored value exactly including capitalization. Do not act on earlier superseded proposals. End with '+second+'.'});
  let queued;for(let n=0;n<100;n++){const s=await r.tui.send('snapshot');queued=s.queuedPromptStrips?.[id]?.items?.find(x=>JSON.stringify(x).includes(second));if(queued)break;await sleep(100)}if(!queued)throw Error('Second ordinary request was not visibly queued');
  await r.capture('ordinary-queue-visible');await Promise.all(web.map(c=>c.capture('ordinary-queue-visible')));
  await r.turn('',first,900000,false);const followup=await r.turn('',second,900000,false);if(!followup.answer.includes(facts.decision))throw Error('Queued followup lost the accepted decision');
  const s=await r.tui.send('snapshot');if(s.queuedPromptStrips?.[id]?.items?.length)throw Error('Queue did not settle');
  return {queuedPrompt:queued,followupDecisionPreserved:true,...await continuity()};
 });await cell('baseline',async()=>{await probe(facts);for(const c of web)await visibleProbe(c);return await continuity()});
 const schedule=process.env.CTXVAL_SWITCH_SCHEDULE?JSON.parse(process.env.CTXVAL_SWITCH_SCHEDULE):[['codex','gpt-6-luna'],['claude-headless','claude-haiku-5-5'],['codex','gpt-6-luna'],['claude-headless','claude-haiku-5-5'],['codex','gpt-6-luna'],['claude-headless','claude-haiku-5-5']];
 result.providerSchedule=schedule;result.excludedCells=process.env.CTXVAL_EXCLUDED_CELLS??null;
 let checkpoint=0,nextSwitch=start;
 while(performance.now()-start<7200000||switched<schedule.length){
  if(switched<schedule.length&&performance.now()>=nextSwitch){
   const [provider,model]=schedule[switched],client=web[switched%2];
   await cell('remote-switch-'+switched,async()=>{const before=await r.binding();await client.select(provider,model);await probe(facts);for(const c of web)await visibleProbe(c);const after=await r.binding();if(before.provider===after.provider&&before.accountProfile===after.accountProfile&&(!before.nativeId||after.nativeId!==before.nativeId))throw Error('Same-account model switch lost native session identity');return {provider,model,before,after,...await continuity()}});switched++;nextSwitch=start+switched*20*60*1000;
   if(switched===3){await cell('idle-home-restart',async()=>{const before=await r.tui.send('snapshot'),focused=before.session.focusedAgentId,bindingBefore=await r.binding();await remote.refresh();await r.restart();await r.command('agent focus '+focused);await probe(facts);for(const c of web)await visibleProbe(c);const bindingAfter=await r.binding();if(!bindingBefore.nativeId||bindingAfter.nativeId!==bindingBefore.nativeId)throw Error('Home restart lost native session identity');return {sessionId:before.session.id,bindingBefore,bindingAfter,...await continuity()}})}
  }else{
   const s=await r.tui.send('snapshot'),id=s.session.focusedAgentId;
   await cell('real-audit-'+checkpoint,async()=>{const turn=await r.turn('Continue the real read-only context-handoff audit. Inspect apps/kernel/src/runtime/state/structured_prompt_dispatch.rs and one relevant test in local_prompt_dispatch_runtime/tests/brief_dispatch_review.rs. Explain one final-envelope protection or retained-brief invariant using exact source references, in at most 120 words. Preserve our nine records unchanged. Do not modify source, read credentials/runtime state or run builds.','CTXVAL_CONTINUITY_AUDIT_'+checkpoint,900000);return {elapsedMs:turn.elapsedMs,...await continuity()}});
   if(checkpoint%5===0){for(const c of web)await visibleProbe(c);await r.capture('continuity-'+checkpoint)}
   const end=Math.min(nextSwitch,performance.now()+120000,start+7200000);while(performance.now()<end){await sleep(Math.min(10000,end-performance.now()));await continuity();const sample=await worker.resources();await r.record('soak-heartbeat-'+Date.now(),{elapsedMs:performance.now()-start,switched,checkpoint,workerResources:sample});if(sample.memoryGiB<2||sample.diskGiB<10)throw Error('Worker resource floor reached');}
   checkpoint++;
  }
 }
 await cell('final-recall',async()=>{await probe(facts);for(const c of web)await visibleProbe(c);return await continuity()});
 result.elapsedMs=performance.now()-start;result.switched=switched;result.meaningfulAuditTurns=checkpoint+3;
 const times=result.latency.map(x=>x.latencyMs).sort((a,b)=>a-b);result.inputVisibleP95Ms=times[Math.ceil(times.length*.95)-1];result.inputVisibleSamples=times.length;
 if(result.elapsedMs<7200000)throw Error('Continuity interval shorter than2hours');result.latencyGate='measurement only (owner C2)';
 result.status='PASS';result.limits='No native cancellation/compaction contract claim (#993). No signed Path1 install or Browser/Computer coverage claim. Actual P1 mutation envelope unit evidence is separate.';
}catch(e){result.status='FAIL';result.firstSeam=String(e?.message??e);await r.capture('continuity-failed').catch(()=>{});await Promise.allSettled(web.map(c=>c.capture('continuity-failed')))}
finally{for(const c of web)await c.close().catch(()=>{});await finish();result.finished=new Date().toISOString();await r.record('result',result)}
if(result.status!=='PASS')process.exitCode=1;
