// MP-08 / MP-10 / MP-11: interruption of real built clients, kernels and an ordinary worker.
import {r,init,finish} from './common.mjs';
try{
 await init({leased:process.env.CTXVAL_LEASED==='1'});
 await r.launch('codex','gpt-6-luna');await r.capture('termination-live-ready');
 await r.record('termination-ready',{driverPid:process.pid,providerTurns:0,scope:'Real hosted built-client/kernel/worker cleanup; no provider behavior or duration acceptance'});
 await new Promise(()=>setInterval(()=>{},1000));
}finally{await finish();}
