// MP-08 / MP-10 / MP-11: M3 nine ordinary/leased pairs and five restart/account variants.
import {r,init,finish,enrollClaude,workload,select,selectAccount,probe} from './common.mjs';
import {webClients} from './web-live.mjs';
const models={codex:'gpt-6-luna','claude-p':'claude-haiku-5-5',opencode:'opencode-go/deepseek-v4.1-flash'};
const alternate={codex:'gpt-6.1-sol','claude-p':'claude-sonnet-5-5',opencode:'opencode/glm-5.3-flash'};
const mode=process.env.CTXVAL_MATRIX??'coverage',leased=process.env.CTXVAL_LEASED==='1';
const result={MP:['MP-08','MP-10','MP-11'],status:'RUNNING',mode,leased,cells:[],started:new Date().toISOString()};
let remote,web=[];
try{
 ({remote}=await init({leased}));await enrollClaude();
 const primary=r.profiles.codex;let secondary;
 if(mode==='restart-account'){
  if(!process.env.CTXVAL_SECOND_CODEX_PROFILE)throw Error('Independent Codex account cell needs CTXVAL_SECOND_CODEX_PROFILE');
  await r.link('codex',process.env.CTXVAL_SECOND_CODEX_PROFILE);secondary=r.profiles.codex;r.profiles.codex=primary;
 }
 const providers=Object.keys(models),cases=mode==='restart-account'
  ?[{source:'codex',target:'codex'},{source:'claude-p',target:'claude-p'},{source:'codex',target:'claude-p',restart:'before-switch'},{source:'claude-p',target:'codex',restart:'after-switch'},{source:'codex',target:'codex',account:true}]
  :providers.flatMap(source=>providers.map(target=>({source,target})));
 for(const item of cases){
  const key=item.source+'-'+item.target+(item.account?'-account':'');if(process.env.CTXVAL_PAIRS&&!process.env.CTXVAL_PAIRS.split(',').includes(key))continue;
  const row={...item,key,status:'FAIL',started:new Date().toISOString(),legs:[]};
  try{
   const facts=await workload(item.source,models[item.source],Number(process.env.CTXVAL_SOURCE_TURNS??15));const before=await r.binding();row.before=before;
   for(const c of web)await c.close();web=[];
   if(item.restart==='before-switch')await r.restart();
   await select(item.target,item.source===item.target?alternate[item.target]:models[item.target]);
   if(item.account)await selectAccount('codex',secondary);
   if(item.restart==='after-switch')await r.restart();
   await probe(facts,false,{addDecision:true});const target=await r.binding();row.legs.push({score:9,total:9,binding:target});
   await select(item.source,models[item.source]);if(item.account)await selectAccount('codex',primary);
   await probe(facts,true);const after=await r.binding();row.legs.push({score:10,total:10,binding:after});
   if(item.source===item.target&&!item.account&&(!before.nativeId||before.nativeId!==target.nativeId||before.nativeId!==after.nativeId))throw Error('Same-account model roundtrip changed native session');
   if(item.account&&(target.accountProfile!==secondary||after.accountProfile!==primary))throw Error('Independent account did not roundtrip');
   if(before.remoteExecution&&JSON.stringify(before.remoteExecution)!==JSON.stringify(after.remoteExecution))throw Error('Worker placement changed');
   // Web changed-action checkpoints run separately; OpenCode never pays for duplicate web probes.
   row.status='PASS';row.limit='Pair core; queue/one-shot and UI checkpoint dimensions remain separate';
  }catch(e){row.firstSeam=String(e?.message??e);await r.capture('matrix-failure-'+key).catch(()=>{});}
  row.finished=new Date().toISOString();result.cells.push(row);await r.record('result',result);if(r.resourceUnsafe)break;
 }
 result.status=result.cells.length&&result.cells.every(x=>x.status==='PASS')?'PASS':'FAIL';
}catch(e){result.status='FAIL';result.firstSeam=String(e?.message??e);}
finally{for(const c of web)await c.close().catch(()=>{});result.finished=new Date().toISOString();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
