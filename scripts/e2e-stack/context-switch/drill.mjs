// MP-08 / MP-10 / MP-11: actual repository work and faults via compiled TUI/web.
import {r,init,finish,select,workload,probe,enrollClaude} from './common.mjs';
import {sleep} from './runtime.mjs';
import {verifyCancelledPromptRecovery} from './cancellation-validation.mjs';
import {webClients} from './web-live.mjs';
import {execFileSync} from 'node:child_process';
import {readFile,writeFile,access} from 'node:fs/promises';
import {summarizeBriefTransition,foreignNativeSession,formatContextValidationFailure} from './context-switch-validation-brief.mjs';
const mode=process.argv[4]??'faults',leased=process.env.CTXVAL_LEASED==='1';
const result={MP:['MP-08','MP-10','MP-11'],mode,leased,status:'RUNNING',started:new Date().toISOString(),cells:[]};let worker,remote,web=[];
let since=Date.now();let facts,ten=false;
async function observe(label){const s=await r.tui.send('snapshot');const data=JSON.parse(execFileSync('python3',[new URL('./history-observe.py',import.meta.url).pathname,r.root,s.session.id,s.session.focusedAgentId],{encoding:'utf8'}));await r.record('history-'+label+'-'+Date.now(),data);const logs=JSON.parse(execFileSync('python3',[new URL('./log-observe.py',import.meta.url).pathname,r.root],{encoding:'utf8'}));await r.record('handoff-log-'+label+'-'+Date.now(),logs);if(worker){const workerLogs=await worker.logs();await r.record('worker-handoff-log-'+label+'-'+Date.now(),workerLogs);data.diagnostics=workerLogs.rows;const workerHistory=await worker.history();await r.record('worker-history-'+label+'-'+Date.now(),workerHistory);data.workerSourceHistories=workerHistory.workerSourceHistories}else data.diagnostics=logs.rows;return data}
async function idle(limit=180000){const begin=Date.now();while(Date.now()-begin<limit){await r.interactions();const s=await r.tui.send('snapshot');if(!s.session.agents.some(a=>a.isProcessing||a.state==='Working'))return s;await sleep(250)}throw Error('Cancelled preparation did not settle')}
async function preparing(marker){const begin=Date.now();while(Date.now()-begin<30000){const s=await r.tui.send('snapshot'),rows=s.agentPanes?.[s.session.focusedAgentId]??[];const current=rows.slice(rows.findLastIndex(x=>x.role==='user'&&x.text?.includes(marker)));if(JSON.stringify({current,status:s.statusLine}).includes('Preparing handoff brief')){await r.capture('preparing-'+Date.now());return s}if(current.some(x=>x.role==='assistant'&&x.text?.includes(marker)))throw Error('Provider dispatched before preparing notice could be observed');await sleep(50)}throw Error('Visible preparing handoff brief notice was not observed')}
async function cell(name,fn){if(process.env.CTXFIX_CELLS&&!process.env.CTXFIX_CELLS.split(',').includes(name))return {cell:name,status:'NOT RUN'};const row={cell:name,status:'FAIL',started:new Date().toISOString()};try{Object.assign(row,await fn());row.status='PASS'}catch(e){row.firstSeam=formatContextValidationFailure(e);await r.capture('failure-'+name).catch(()=>{})}row.finished=new Date().toISOString();result.cells.push(row);await r.record('result',result);if(r.resourceUnsafe)throw Error('Measured shared-builder resource floor reached');return row}
async function cycle(provider,model,{client=null}={}){const before=await observe('before-switch'),source=await r.binding();if(client){await client.select(provider,model);await client.probe(facts,ten)}else{await select(provider,model);await probe(facts,ten)}const target=await r.binding(),after=await observe('after-switch'),transition=summarizeBriefTransition(before,after);await r.record('switch-'+Date.now(),{source,target,foreign:foreignNativeSession(source,target),transition});if(transition.watermarkRegressed)throw Error('Durable brief watermark regressed');return {source,target,transition,foreign:foreignNativeSession(source,target)}}
const topics=[
 ['docs/INTER_PROVIDER_CONTEXT_SWITCHING_PLAN.md','Compare each promised native-session invariant against its implementation, quoting only relevant project-owned source.'],
 ['apps/kernel/src/runtime/state/context_handoff/builder.rs','Audit how prior user prompts and assistant outputs are bounded, and show the exact rendering policy.'],
 ['apps/kernel/src/runtime/state/context_handoff/brief_refresh.rs','Audit cancellation checks and the first model-rejection retry. Include exact implementation excerpts and source references.'],
 ['apps/kernel/src/runtime/state/local_prompt_dispatch_runtime.rs','Find the detached preparation continuation and cancellation settlement. Explain how it prevents duplicate submission.'],
 ['apps/kernel/src/runtime/state/agent_config_runtime_state.rs','Trace a provider/account/model update and stale-run retirement. List state transitions and source excerpts.'],
 ['apps/kernel/src/provider/launch_contract.rs','Audit native resume policy for same-provider model changes versus cross-provider changes.'],
 ['apps/kernel/src/runtime/state/context_handoff/protection.rs','Trace observation-cache policy revalidation and the persisted watermark invalidation boundary.'],
 ['apps/kernel/src/history/handoff_brief.rs','Audit persistence transaction ordering and nonregressing watermark handling.'],
 ['apps/kernel/src/provider/opencode.rs','Trace official OpenCode session start/resume admission and model IDs; inspect the responsibility modules.'],
 ['apps/kernel/src/provider/codex.rs','Trace official Codex app-server session creation, model changes, and safe native-thread reuse.'],
 ['apps/kernel/src/runtime/state/remote_prompt_worker_submission_runtime.rs','Audit leased-worker dispatch retry and acknowledgement ordering against history continuity.'],
 ['apps/kernel/src/app/remote_agent_binding.rs','Audit exact ordinary worker binding and recovery; identify fields keeping worker placement stable.'],
 ['apps/cli/src/agent-config-command-handlers.ts','Trace real TUI provider/account/model controls and their visible acknowledgement.'],
 ['apps/kernel/src/provider/runtime_run.rs','Audit runtime/native session identity separation and provider lifecycle.'],
 ['docs/PROTOCOL.md','Compare native provider TUI context, permission and cancellation contracts against the audited source.'],
 ['docs/ARCHITECTURE.md','Compare kernel history authority versus relay transport authority, with exact project excerpts.'],
];
async function audit(n){const [file,task]=topics[n%topics.length];return r.turn(`Continue the actual read-only context-switch audit. Inspect ${file}; if moved, locate its responsibility with rg --files. ${task} Read needed implementation and existing tests; do not execute cargo. Give a detailed evidence-based answer with exact source excerpts where necessary, at most 350 words, limited to the named responsibility and its directly relevant tests. Use at most four read/search actions. Preserve the nine original audit records unchanged and do not repeat their JSON or assistant-only proposal. Do not write source/files or read account, credential or runtime state.`, 'CTXVAL_R4_AUDIT_'+n,900000)}
try{
 ({worker,remote}=await init({leased}));
 await enrollClaude();facts=await workload('claude-p','claude-haiku-5-5',Number(process.env.CTXVAL_SOURCE_TURNS??3));
 if(mode==='faults'||mode==='worker'){let n=0;while((await observe('fault-history-fill')).contentBytes<=116280&&n<40)await audit(n++);if((await observe('fault-ready')).contentBytes<=116280)throw Error('Actual fault workload never crossed the handoff byte budget');}
 await r.record('workload-manifest',{repository:'https://github.com/charioxai/chariox.git',readOnlyTask:'audit context-handoff cancellation',normalization:'completed semicolon suffix; sha256 label prefix; NFC',providers:['Codex official app-server','Claude official CLI'],models:['gpt-6-luna','claude-haiku-5-5']});
 web=await webClients(r,remote);
 if(mode==='faults'||mode==='worker'){
  await cell('CX-03-cancel-mid-switch',async()=>{
   const before=await observe('cancel-before');
   await select('codex','gpt-6-luna');
   const marker='CTXVAL_R4_CANCELLED_'+Date.now();
   await r.tui.send('submit_prompt',{prompt:'Read the source cancellation continuation and explain its ownership check. End with '+marker+'.'});
   const pending=await preparing(marker);
   await Promise.all(web.map(c=>c.capture('preparing-cancel')));
   await r.command('stop');
   return verifyCancelledPromptRecovery({
    agentId:pending.session.focusedAgentId,marker,settle:idle,wait:sleep,snapshot:()=>r.tui.send('snapshot'),
    checkFold:async()=>{
     const after=await observe('cancel-after');
     if(before.brief?.sha256!==after.brief?.sha256||before.brief?.coveredThroughSequence!==after.brief?.coveredThroughSequence)throw Error('Cancelled fold persisted');
    },
    recover:async()=>{
     try{await probe(facts);for(const c of web)await c.probe(facts)}
     finally{await observe('cancel-recovery');await Promise.all(web.map(c=>c.capture('cancel-recovery')))}
    }
   });
  });
  await cell('CX-03-target-model-rejection',async()=>{await select('claude-p','claude-haiku-5-5');await audit(0);await r.command('config set history.handoff.codex_brief_model ctxval-invalid-brief-model-r4');const first=await cycle('codex','gpt-6-luna',{client:web[0]});await select('claude-p','claude-haiku-5-5');await audit(1);const second=await cycle('codex','gpt-6-luna',{client:web[1]});const observed=await observe('rejection-final');const fallback=observed.diagnostics.filter(x=>x.message==='the brief model is unavailable; briefing with the target model');const updates=observed.diagnostics.filter(x=>x.message==="updated the agent's handoff brief");if(fallback.length!==1||!updates.some(x=>x.fields.utility_calls===2))throw Error('Official model rejection/retry-once was not proved');await r.command('config unset history.handoff.codex_brief_model');return {first,second,recoveryRecall:9,fallbackCount:fallback.length,utilityCalls:updates.map(x=>x.fields.utility_calls)}});
  await r.command('config unset history.handoff.codex_brief_model');
  await cell('CX-03-relay-outage-switch',async()=>{if(!remote)throw Error('Owned shaped namespace outage requires a real remote client');const baseline=await observe('outage-baseline');if(!baseline.brief){await select('codex','gpt-6-luna');await r.turn('Continue the read-only audit: inspect apps/kernel/src/runtime/state/context_handoff.rs and explain one dispatch ownership boundary with a source reference. Do not edit files or read credentials/runtime state; preserve our existing audit records unchanged.','CTXVAL_OUTAGE_BASELINE');if(!(await observe('outage-stored-baseline')).brief)throw Error('Outage has no prior stored fold')}await select('claude-p','claude-haiku-5-5');await audit(2);await web[1].select('codex','gpt-6-luna');const marker='CTXVAL_R4_OUTAGE_'+Date.now();const input=web[1].page.locator('[data-terminal-prompt]:not(:disabled)');await input.fill('Without tools recall our existing audit records as JSON with exactly '+Object.keys(facts).join(', ')+'. End with '+marker+'.');await input.press('Enter');await preparing(marker);await remote.outage();const turn=await r.turn('',marker,240000,false);const {scoreContextValidationRecall}=await import('./context-switch-validation-records.mjs');const score=scoreContextValidationRecall(turn.state.agentPanes[turn.state.session.focusedAgentId],marker,facts);await r.record('outage-score',score);if(score.score!==9||score.toolCount)throw Error('Outage recovery recall failed');const after=await observe('outage-after');const ids=after.prompts.map(x=>x.id).filter(Boolean);if(new Set(ids).size!==ids.length)throw Error('Prompt history contains duplicate IDs');await Promise.all(web.map(c=>c.capture('outage-recovered')));return {outageMs:30000,score,promptIdsUnique:true}});
  if(!process.env.CTXFIX_CELLS||process.env.CTXFIX_CELLS.split(',').includes('CX-03-native-bridge'))result.cells.push({cell:'CX-03-native-bridge',status:'NOT RUN',note:'MP-08 / MP-10 / MP-11: #978 is merged; the dedicated real native-hook drill is required, and structured Codex/OpenCode cells do not prove that bridge.'});
  if(mode==='worker'){
  let n=0;while((await observe('history-fill')).contentBytes<=116280&&n<40){await audit(n++);}
  await cell('CX-02-short-worker-incremental',async()=>{const fill=await observe('before-incremental');if(fill.contentBytes<=116280)throw Error('Actual history did not exceed the Codex handoff budget');const first=await cycle('codex','gpt-6-luna',{client:web[0]});await r.turn("Add one accepted user decision: tenth_decision is exactly 'validate queue once'. Preserve all prior audit records. Reply briefly without tools.",'CTXVAL_R4_TENTH');ten=true;await audit(n++);const returned=await cycle('claude-p','claude-haiku-5-5',{client:web[1]});await remote.refresh();await r.restart();const last=await cycle('codex','gpt-6-luna');if(!last.transition.hasStoredBrief)throw Error('Incremental restart has no durable brief');return {first,returned,last,meaningfulAuditTurns:n,historyBytes:fill.contentBytes}});

  }

 }else if(mode==='long'){
  since=Date.now();
  let n=0,nextSwitch=since+20*60*1000,switches=0,provider='claude-p';const end=since+3*60*60*1000;
  while(Date.now()<end||switches<9){
   const resources=JSON.parse(execFileSync('python3',['-c',"import json,shutil; m=dict(line.split(':',1) for line in open('/proc/meminfo')); print(json.dumps({'diskFreeGiB':shutil.disk_usage('/').free/2**30,'memAvailableGiB':int(m['MemAvailable'].split()[0])/2**20}))"],{encoding:'utf8'}));await r.record('resources-'+n,resources);if(resources.diskFreeGiB<40||resources.memAvailableGiB<12)throw Error('Measured resource floor reached');
   if(Date.now()>=nextSwitch){if(process.env.CTXVAL_CODEX_PAUSE_FILE){let held=false;try{await access(process.env.CTXVAL_CODEX_PAUSE_FILE);held=true}catch{}if(held){nextSwitch=Date.now()+90000;await r.record('codex-short-cell-gate',{held:true,sourceAuditContinues:true});continue;}}provider=provider==='claude-p'?'codex':'claude-p';const model=provider==='codex'?'gpt-6-luna':'claude-haiku-5-5';await cell('CX-02-switch-'+switches,async()=>cycle(provider,model,{client:web[switches%2]}));switches++;nextSwitch=Date.now()+20*60*1000;if(switches===4&&worker){await cell('CX-LEASE-worker-only-restart',async()=>{const before=await r.binding(),restart=await worker.restart();await remote.refresh();await probe(facts,ten);for(const c of web)await c.probe(facts,ten);return {before,after:await r.binding(),restart,recall:ten?10:9,homeKernelRestarted:false};});}if(!ten){await r.turn("Add one accepted user decision: tenth_decision is exactly 'validate queue once'. Preserve all prior audit records. Reply briefly without tools.",'CTXVAL_R4_TENTH');ten=true}await r.record('long-checkpoint',{elapsedMs:Date.now()-since,switches,nextSwitch});
   }else{
    await cell('CX-02-work-'+n,async()=>{const turn=await audit(n);return {elapsedMs:turn.elapsedMs}});n++;
    const deadline=Math.min(nextSwitch,Date.now()+90*1000);while(Date.now()<deadline){await sleep(Math.min(15000,deadline-Date.now()));const s=await r.tui.send('snapshot');if(s.daemonDisconnected)throw Error('TUI disconnected during long-session stability interval')}
   }
  }
  await cell('CX-02-incremental-brief',async()=>{const last=await observe('long-final');if(!last.brief)throw Error('Long-history session has no persisted brief');const decision=last.brief.text.includes('validate queue once'),elapsed=Date.now()-since;if(elapsed<10800000)throw Error('Session shorter than 3 hours');if(!decision)throw Error('Incremental brief lost accepted tenth decision');return {elapsedMs:elapsed,meaningfulAuditTurns:n,switches,brief:last.brief}});
 }else if(mode==='incremental'){
  let n=0;while((await observe('history-fill')).contentBytes<=116280&&n<40){await audit(n++);}
  await cell('CX-02-short-worker-incremental',async()=>{const fill=await observe('before-incremental');if(fill.contentBytes<=116280)throw Error('Actual history did not exceed the Codex handoff budget');const first=await cycle('codex','gpt-6-luna',{client:web[0]});await r.turn("Add one accepted user decision: tenth_decision is exactly 'validate queue once'. Preserve all prior audit records. Reply briefly without tools.",'CTXVAL_R4_TENTH');ten=true;await audit(n++);const returned=await cycle('claude-p','claude-haiku-5-5',{client:web[1]});await remote.refresh();await r.restart();const last=await cycle('codex','gpt-6-luna');if(!last.transition.hasStoredBrief)throw Error('Incremental restart has no durable brief');return {first,returned,last,meaningfulAuditTurns:n,historyBytes:fill.contentBytes}});
 }else if(mode==='control'){
  for(const [provider,model,client]of [['codex','gpt-6-luna',web[0]],['claude-p','claude-haiku-5-5',web[1]]])await cell('short-'+provider,async()=>cycle(provider,model,{client}));
 }
 result.finished=new Date().toISOString();result.elapsedMs=Date.now()-since;result.status=result.cells.every(x=>x.status==='PASS')?'PASS':'FAIL';
}catch(e){result.status='FAIL';result.firstSeam=formatContextValidationFailure(e);result.finished=new Date().toISOString();await r.capture('fatal-failure').catch(()=>{})}
finally{await r.record('result',result);for(const c of web)await c.close().catch(()=>{});await finish()}
if(result.status!=='PASS')process.exitCode=1;
