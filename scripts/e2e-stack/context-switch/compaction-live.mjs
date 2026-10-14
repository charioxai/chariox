// MP-08 / MP-10 / MP-11: real window evidence, downshift ordering and negative controls.
import {r,init,finish,enrollClaude,workload,probe,select} from './common.mjs';
import {webClients} from './web-live.mjs';
import {nativeWindow} from './native-window.mjs';
import {execFileSync} from 'node:child_process';
import {sleep} from './runtime.mjs';
import {validateCompactionOutage} from './control-validation.mjs';
const mode=process.env.CTXVAL_COMPACTION??'positive',result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',mode,cells:[],started:new Date().toISOString()};
const source=process.env.CTXVAL_COMPACTION_SOURCE??'claude-sonnet-5-5[1m]',target=process.env.CTXVAL_COMPACTION_TARGET??'claude-haiku-5-5';let web=[],remote,outage,worker;
async function logs(label){let data=JSON.parse(execFileSync('python3',[new URL('./log-observe.py',import.meta.url).pathname,r.root],{encoding:'utf8'}));if(worker){const home=data;data=await worker.logs();await r.record('home-compaction-log-'+label,home);}await r.record('compaction-log-'+label,data);return data.rows.filter(x=>/compacted the session before|failed to compact the session before/.test(x.message));}
try{
 ({remote,worker}=await init({leased:process.env.CTXVAL_LEASED==='1'}));await enrollClaude();
 result.windows=[await nativeWindow(r,source,{worker}),await nativeWindow(r,target,{worker})];
 if(mode==='positive'||mode==='outage'){
  const targetWindow=result.windows[1].windowTokens;if(result.windows[0].windowTokens<=targetWindow)throw Error('Official source window is not larger; choose a real supported smaller-window model');const facts=await workload('claude-p',source,15);
  let before=await r.binding(),iterations=0;
  while((before.runs.find(x=>x.provider===before.provider)?.contextTokens??0)<=targetWindow*.75&&iterations<30){
   const files=['apps/kernel/src/runtime/state/agent_config_runtime_state/tests/profile_compaction.rs','apps/kernel/src/runtime/state/context_handoff/builder.rs','apps/kernel/src/app/provider_output_claude_native/tests.rs','docs/INTER_PROVIDER_CONTEXT_SWITCHING_PLAN.md'];
   await r.turn('Continue the real read-only context-switch audit. Inspect '+files[iterations%files.length]+'. Explain one previously unexamined assertion about source compaction, queued work, native identity or replay; cite exact source. Read the necessary test body and implementation. Keep all nine agreed records unchanged. Do not edit source, run builds or read credential/runtime state.','CX04_WINDOW_AUDIT_'+iterations,900000);before=await r.binding();iterations++;
  }
  const tokens=before.runs.find(x=>x.provider===before.provider)?.contextTokens;if(!(tokens>targetWindow*.75))throw Error('Real source API input never crossed75% of the verified target window');
  if(result.windows[0].windowTokens<=targetWindow)throw Error('Official source window is not larger');
  web=await webClients(r,remote);const peerState=process.env.CTXVAL_COMPACTION_PEER==='1'?await (await import('./compaction-peer.mjs')).prepareCompactionPeer(r):null;if(peerState)await web[0].focus(peerState.sourceId);const baseline=await logs('before');
  // Begin normal web selection without awaiting compaction, then queue through the real TUI.
  const selectedAt=Date.now();if(mode==='outage'){if(!r.providerNet)throw Error('Scoped official Claude proxy required');outage=r.providerNet.outage(310000);}let selectionError,selectionSettled=false;const selection=web[0].select('claude-p',target,{modelOnly:true}).catch(error=>{selectionError=error;}).finally(()=>{selectionSettled=true;});let compacting=false;
  for(let n=0;n<600;n++){if(selectionError)throw selectionError;const s=await r.tui.send('snapshot');if(JSON.stringify(s.agentPanes).includes('Compacting the Claude session')){compacting=true;break;}await sleep(100);}
  if(!compacting)throw Error('Visible source compaction was never observed');await r.capture('compacting');for(const c of web)await c.capture('compacting');
  if(mode==='outage'){
   await selection;if(selectionError)throw selectionError;const settledAt=Date.now();await outage;outage=null;const after=await r.binding(),events=await logs('outage-after');
   const elapsedMs=settledAt-selectedAt;validateCompactionOutage({baseline,events,elapsedMs,sourceRunId:before.runs.find(x=>x.provider===before.provider)?.id.split(':').at(-1),sourceModel:before.model,targetModel:target});
   await probe(facts,false,{addDecision:true});await probe(facts,true);for(const c of web){await c.probe(facts,true);await c.capture('outage-fallback-recall');}
   result.cells.push({cell:'300s-source-outage',status:'PASS',contextTokens:tokens,targetWindow,elapsedMs,relayResponseBudgetMs:20000,outageMs:310000,before,after,events,ordinaryFallbackRecall:10});result.status='PASS';result.limits='300s source failure/fallback only; queued work and unrelated-agent interactions measured separately';
   await r.record('result',result);
  }else{
  const marker='CX04_QUEUED_'+Date.now();await r.tui.send('submit_prompt',{prompt:'Without tools recall our nine agreed audit records as JSON with exactly '+Object.keys(facts).join(', ')+'. End with '+marker+'.'});await r.capture('queued-once');
  if(peerState)result.peer=await (await import('./compaction-peer.mjs')).approveCompactionPeer(r,peerState,()=>!selectionSettled,id=>web[0].focus(id));
  await selection;if(selectionError)throw selectionError;const turn=await r.turn('',marker,900000,false);const {scoreContextValidationRecall}=await import('./context-switch-validation-records.mjs');const score=scoreContextValidationRecall(turn.state.agentPanes[turn.state.session.focusedAgentId],marker,facts);
  if(score.score!==9||score.toolCount)throw Error('Queued no-tool recall failed');const state=await r.tui.send('snapshot'),rows=state.agentPanes[state.session.focusedAgentId];if(rows.filter(x=>x.role==='user'&&x.text?.includes(marker)).length!==1)throw Error('Queued task was duplicated');
  const after=await r.binding(),compactions=await logs('after');if(compactions.length!==baseline.length+1||!/compacted the session before/.test(compactions.at(-1).message))throw Error('Source compaction success was not logged exactly once');
  if(result.peer&&result.peer.replyRecordedAtMs>=compactions.at(-1).atMs)throw Error('Peer permission was not answered inside the actual source compaction interval');
  if(!before.nativeId||!after.nativeId||before.nativeId!==after.nativeId)throw Error('Successful source compaction changed native UUID');
  await select('claude-p',source);await probe(facts,false,{addDecision:true});await probe(facts,true);for(const c of web){await c.probe(facts,true);await c.capture('compacted-recall');}
  result.cells.push({cell:'positive',status:'PASS',before,after,contextTokens:tokens,targetWindow,score,iterations,queueOnce:true,compactions});
  }
 }else{
  for(const name of (process.env.CTXVAL_CONTROL_NAMES??'below-threshold,equal-window,upshift,provider-change,headless,no-live-run').split(',')){
   const provider=name==='headless'?'claude-headless':'claude-p';const facts=await workload(provider,name==='upshift'?target:source,1);const before=await logs(name+'-before');
   if(name==='no-live-run')await r.restart();
   if(name==='provider-change')await select('codex','gpt-6-luna');else await select(provider,name==='equal-window'?'claude-sonnet-5-5':name==='upshift'?source:target);
   await probe(facts);const after=await logs(name+'-after');if(after.length!==before.length)throw Error('Unexpected structured precompaction: '+name);result.cells.push({cell:name,status:'PASS',precompactions:0});await r.record('result',result);
  }
 }
 result.status='PASS';result.limits??='Scoped live subcells; capped-environment, unrelated-agent interactions and leased source logs remain separate';
}catch(e){result.firstSeam=String(e?.message??e);await r.capture('compaction-failure').catch(()=>{});for(const c of web)await c.capture('compaction-failure').catch(()=>{});}
finally{if(outage)await outage.catch(()=>{});for(const c of web)await c.close().catch(()=>{});result.finished=new Date().toISOString();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
