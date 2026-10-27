// MP-08 / MP-10 / MP-11: real leased native resume after native hook metadata persistence.
import {r,init,finish,workload,probe,enrollClaude} from './common.mjs';
import {webClients} from './web-live.mjs';
const result={MP:['MP-08','MP-10','MP-11'],status:'RUNNING',scope:'short final-source native identity repair, not full CX-05 or 2h acceptance',started:new Date().toISOString(),cells:[]};
let worker,remote,web=[],facts;
async function cell(name,fn){const row={cell:name,status:'FAIL',started:new Date().toISOString()};try{Object.assign(row,await fn());row.status='PASS'}catch(e){row.firstSeam=String(e?.message??e);result.cells.push(row);throw e}row.finished=new Date().toISOString();result.cells.push(row);await r.record('result',result)}
try{
 ({worker,remote}=await init({leased:true}));await enrollClaude();facts=await workload('claude-headless','claude-haiku-5-5',3);web=await webClients(r,remote);
 const initial=await r.binding();if(!initial.nativeId)throw Error('Native baseline has no confirmed session ID');
 await cell('same-account-upsizing-after-full-web-profile-selection',async()=>{await web[0].select('claude-headless','claude-sonnet-5-5');await probe(facts);for(const c of web)await c.probe(facts);const after=await r.binding();if(after.nativeId!==initial.nativeId)throw Error('Native session replaced on same-account upsizing');return {before:initial,after,nativeSessionRetained:true,recall:9}});
 await cell('same-account-model-only-downshift',async()=>{const before=await r.binding();await web[1].select('claude-headless','claude-haiku-5-5',{modelOnly:true});await probe(facts);for(const c of web)await c.probe(facts);const after=await r.binding();if(after.nativeId!==before.nativeId)throw Error('Native session replaced on model-only downshift');return {before,after,nativeSessionRetained:true,recall:9}});
 await cell('idle-home-restart-native-resume',async()=>{const before=await r.binding(),state=await r.tui.send('snapshot'),focused=state.session.focusedAgentId;await remote.refresh();await r.restart();await r.command('agent focus '+focused);await probe(facts);for(const c of web)await c.probe(facts);const after=await r.binding();if(after.nativeId!==before.nativeId)throw Error('Native session replaced after idle home restart');return {before,after,nativeSessionRetained:true,recall:9}});
 result.status='PASS';result.limits='Real official Claude, actual built TUI/kernel/relay and hosted real app at DPR1/2. No Codex/OpenCode/cancellation/large-context/2h claim.';
}catch(e){result.status='FAIL';result.firstSeam=String(e?.message??e);await r.capture('native-resume-failed').catch(()=>{});for(const c of web)await c.capture('native-resume-failed').catch(()=>{})}
finally{for(const c of web)await c.close().catch(()=>{});await finish();result.finished=new Date().toISOString();await r.record('result',result)}
if(result.status!=='PASS')process.exitCode=1;
