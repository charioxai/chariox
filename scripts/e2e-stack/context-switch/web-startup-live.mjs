// MP-08 / MP-10 / MP-11: real browser partial-startup cleanup, no provider turns/UI acceptance.
import {r,init,finish} from './common.mjs';
import {webClients} from './web-live.mjs';
const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',providerTurns:0,scope:'Exception cleanup of real built-client/kernel/browser stack only; not UI acceptance'};
let web=[];
try{
 const {remote}=await init();await r.launch('codex','gpt-6-luna');
 try{web=await webClients(r,remote);if(process.env.CTXVAL_EXPECT_MISSING_FFMPEG==='1')throw Error('Expected missing video dependency was not observed');result.status='PASS';}
 catch(error){if(process.env.CTXVAL_EXPECT_MISSING_FFMPEG==='1'&&/Executable doesn't exist at .*ffmpeg/.test(String(error))){result.status='PASS';result.expectedFailureClass='Missing ffmpeg before first real web page';}else throw error;}
}catch(error){result.firstSeam=String(error?.message??error);}
finally{for(const c of web)await c.close();await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
