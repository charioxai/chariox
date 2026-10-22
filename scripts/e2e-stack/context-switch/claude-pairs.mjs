// MP-08 / MP-10 / MP-11: five real provider round trips, real TUI/web, exact recall.
import {r,init,finish,enrollClaude,workload,select,probe} from './common.mjs';
import {webClients} from './web-live.mjs';
import {foreignNativeSession,formatContextValidationFailure} from './context-switch-validation-brief.mjs';
const models={codex:'gpt-6-luna','claude-p':'claude-haiku-5-5',opencode:'opencode-go/deepseek-v4.1-flash'};
const pairs=[['codex','claude-p','codex'],['claude-p','codex','claude-p'],['claude-p','claude-p','claude-p'],['claude-p','opencode','claude-p'],['opencode','claude-p','opencode']];
const result={MP:['MP-08','MP-10','MP-11'],status:'RUNNING',leased:process.env.CTXVAL_LEASED==='1',cells:[],started:new Date().toISOString()};let web=[],remote;
try{
 ({remote}=await init({leased:result.leased}));await enrollClaude();
 for(const providers of pairs){
  if(process.env.CTXVAL_PAIRS&&!process.env.CTXVAL_PAIRS.split(',').includes(providers[0]+'-'+providers[1]))continue;
  const row={providers,status:'FAIL',legs:[]};
  try{
   const facts=await workload(providers[0],providers.every(x=>x==='claude-p')?'claude-sonnet-5-5':models[providers[0]],Number(process.env.CTXVAL_SOURCE_TURNS??3));
   for(const c of web)await c.close();web=await webClients(r,remote);
   let source=await r.binding();row.source=source;
   for(let leg=1;leg<3;leg++){
    const provider=providers[leg],model=providers.every(x=>x==='claude-p')&&leg===2?'claude-sonnet-5-5':models[provider];
    const client=web[leg-1];await client.select(provider,model);await probe(facts,leg===2);const target=await r.binding();const foreign=foreignNativeSession(source,target);
    if(!foreign.verified)throw Error('Native IDs not both observed');if(foreign.reused)throw Error('Foreign native session ID reused');if(leg===2&&row.source.provider!==providers[1]&&row.source.nativeId===target.nativeId)throw Error('Cross-provider return reused the stale native conversation');
    if(source.provider===target.provider&&source.accountProfile===target.accountProfile&&source.nativeId!==target.nativeId)throw Error('Same-provider model change replaced its native conversation');
    const score=await client.probe(facts,leg===2);row.legs.push({source,target,foreign,score});source=target;
    if(leg===1)await r.turn("Add one accepted user decision: tenth_decision is exactly 'validate queue once'. Preserve all prior audit records. Reply briefly without tools.",'CTXVAL_PAIR_TENTH');
   }
   row.status='PASS';
  }catch(e){row.firstSeam=formatContextValidationFailure(e);await r.capture('claude-pair-failure-'+result.cells.length).catch(()=>{});await Promise.allSettled(web.map(c=>c.capture('pair-failure-'+result.cells.length))); }
  result.cells.push(row);await r.record('result',result);if(r.resourceUnsafe)throw Error('Shared-builder floor reached');
 }
 result.status=result.cells.every(x=>x.status==='PASS')?'PASS':'FAIL';
}catch(e){result.status='FAIL';result.firstSeam=formatContextValidationFailure(e);}
finally{for(const c of web)await c.close().catch(()=>{});result.finished=new Date().toISOString();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
