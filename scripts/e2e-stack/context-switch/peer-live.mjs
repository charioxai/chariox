// MP-08 / MP-10 / MP-11: supplementary real-provider audit preflight, no concurrency acceptance.
import {r,init,enrollClaude,finish,workload} from './common.mjs';
import {nativeWindow} from './native-window.mjs';
import {webClients} from './web-live.mjs';
import {prepareCompactionPeer} from './compaction-peer.mjs';
const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',scope:'Actual provider/permission preflight only; no compaction or utility concurrency measured'};
let web=[];try{const leased=process.env.CTXVAL_LEASED==='1',{remote,worker}=await init({leased});await enrollClaude();if(process.env.CTXVAL_ENROLL_HISTORY==='1')result.history=await (await import('./enrollment-history.mjs')).enrollDormantHistory(r);if(process.env.CTXVAL_PEER_NATIVE_WINDOWS==='1'){await nativeWindow(r,'claude-sonnet-5-5[1m]',{worker});await nativeWindow(r,'claude-haiku-4-5',{worker});}if(leased){await workload('claude-p','claude-haiku-5-5',1);web=await webClients(r,remote);}else await r.launch('claude-p','claude-haiku-5-5');const peer=await prepareCompactionPeer(r);await r.tui.send('interaction_submit',{choiceIndex:0});await r.turn('',peer.marker,180000,false);result.status='PASS';result.peer=peer;}
catch(e){result.firstSeam=String(e?.message??e);await r.capture('peer-preflight-failure').catch(()=>{});}
finally{await r.record('result',result);for(const c of web)await c.close().catch(()=>{});await finish();}
if(result.status!=='PASS')process.exitCode=1;
