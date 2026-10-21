// MP-08 / MP-10 / MP-11: M3 nine ordinary/leased pairs and five restart/account variants.
import {r,init,finish,enrollClaude,workload,select,selectAccount,probe} from './common.mjs';
import {webClients} from './web-live.mjs';
import {validateAccountNativeSwitch} from './control-validation.mjs';
const models={codex:'gpt-6-luna','claude-p':'claude-haiku-5-5',opencode:process.env.CTXVAL_OPENCODE_MODEL??'opencode-go/deepseek-v4.1-flash'};
const alternate={codex:'gpt-6.1-sol','claude-p':'claude-sonnet-5-5',opencode:'opencode/glm-5.3-flash'};
const mode=process.env.CTXVAL_MATRIX??'coverage',leased=process.env.CTXVAL_LEASED==='1';
const result={MP:['MP-08','MP-10','MP-11'],status:'RUNNING',mode,leased,cells:[],started:new Date().toISOString()};
let remote,web=[],accountBlocker;
try{
 ({remote}=await init({leased}));await enrollClaude();
 const primary=r.profiles.codex;let secondary;
 if(mode==='restart-account'){
  try{if(!process.env.CTXVAL_SECOND_CODEX_PROFILE)throw Error('Independent Codex account cell needs CTXVAL_SECOND_CODEX_PROFILE');await r.link('codex',process.env.CTXVAL_SECOND_CODEX_PROFILE);secondary=r.profiles.codex;}catch(e){accountBlocker=String(e?.message??e);await r.record('independent-account-blocker',{firstSeam:accountBlocker,ownerAction:'coordinator repair the disposable product-linked Codex account; no manual login/refresh'});}finally{r.profiles.codex=primary;}
 }
 const providers=Object.keys(models),cases=mode==='restart-account'
  ?[{source:'codex',target:'codex'},{source:'claude-p',target:'claude-p'},{source:'codex',target:'claude-p',restart:'before-switch'},{source:'claude-p',target:'codex',restart:'after-switch'},{source:'codex',target:'codex',account:true}]
  :providers.flatMap(source=>providers.map(target=>({source,target})));
 for(const item of cases){
  const key=item.source+'-'+item.target+(item.account?'-account':'');if(process.env.CTXVAL_PAIRS&&!process.env.CTXVAL_PAIRS.split(',').includes(key))continue;
  const row={...item,key,status:'FAIL',started:new Date().toISOString(),legs:[]};
  if(item.account&&accountBlocker){row.status='BLOCKED(coordinator: independent disposable Codex account)';row.firstSeam=accountBlocker;result.cells.push(row);await r.record('result',result);continue;}
  try{
   const facts=await workload(item.source,models[item.source],Number(process.env.CTXVAL_SOURCE_TURNS??15));const before=await r.binding();row.before=before;
   for(const c of web)await c.close();web=[];
   if(item.restart==='before-switch')await r.restart();
   await select(item.target,item.source===item.target&&!item.account?alternate[item.target]:models[item.target]);
   if(item.account)await selectAccount('codex',secondary);
   if(item.restart==='after-switch')await r.restart();
   if(mode==='restart-account'){
    const marker='CX01_NEXT_AUDIT_'+Date.now(),pending=probe(facts,false,{addDecision:true});let active=false;
    for(let n=0;n<200;n++){const x=await r.tui.send('snapshot');if(x.session.agents.find(a=>a.id===x.session.focusedAgentId)?.isProcessing){active=true;break;}await new Promise(r=>setTimeout(r,100));}
    if(!active)throw Error('First recall never became active for queued work');
    await r.tui.send('submit_prompt',{prompt:'Continue our genuine read-only source audit. Read apps/kernel/src/provider/launch_contract.rs and cite one native-resume invariant. Preserve the ten accepted records and do not edit source or read credentials/runtime state. End with '+marker+'.'});
    await r.capture('CX01-queued-'+key);await pending;await r.turn('',marker,900000,false);
    const x=await r.tui.send('snapshot'),rows=x.agentPanes[x.session.focusedAgentId];if(rows.filter(x=>x.role==='user'&&x.text?.includes(marker)).length!==1)throw Error('Queued audit did not run exactly once');row.queueOnce=true;
   }else await probe(facts,false,{addDecision:true});const target=await r.binding();row.legs.push({score:9,total:9,binding:target});
   await select(item.source,models[item.source]);if(item.account)await selectAccount('codex',primary);
   await probe(facts,true);const after=await r.binding();if(!before.nativeId||!target.nativeId||!after.nativeId)throw Error('Native IDs were not observed');if(item.source!==item.target&&before.nativeId===target.nativeId)throw Error('Foreign native ID was reused');row.legs.push({score:10,total:10,binding:after});
   if(item.source===item.target&&!item.account&&(!before.nativeId||before.nativeId!==target.nativeId||before.nativeId!==after.nativeId))throw Error('Same-account model roundtrip changed native session');
   if(item.account)validateAccountNativeSwitch({before,target,after,primary,secondary});
   if(before.remoteExecution&&JSON.stringify(before.remoteExecution)!==JSON.stringify(after.remoteExecution))throw Error('Worker placement changed');
   // Web changed-action checkpoints run separately; OpenCode never pays for duplicate web probes.
   row.status='PASS';row.limit='Pair core; queue/one-shot and UI checkpoint dimensions remain separate';
  }catch(e){row.firstSeam=String(e?.message??e);await r.capture('matrix-failure-'+key).catch(()=>{});if(/429|response_too_many_failed_attempts|rate_limit_exceeded|credit balance|insufficient funds|payment required/i.test(row.firstSeam)){result.cells.push(row);throw e;}}
  row.finished=new Date().toISOString();result.cells.push(row);await r.record('result',result);if(r.resourceUnsafe)break;
 }
 result.status=result.cells.length&&result.cells.every(x=>x.status==='PASS')?'PASS':'FAIL';
}catch(e){result.status='FAIL';result.firstSeam=String(e?.message??e);}
finally{for(const c of web)await c.close().catch(()=>{});result.finished=new Date().toISOString();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
