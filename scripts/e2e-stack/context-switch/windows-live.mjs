// MP-08 / MP-10 / MP-11: official native window evidence without a provider turn.
import {r,init,finish,enrollClaude} from './common.mjs';
import {nativeWindow} from './native-window.mjs';
const result={MP:['MP-08','MP-10','MP-11'],status:'FAIL',windows:[]};
try{
 await init();await enrollClaude();
 for(const model of (process.env.CTXVAL_WINDOW_MODELS??'claude-sonnet-5-5[1m],claude-haiku-5-5').split(','))result.windows.push(await nativeWindow(r,model));
 result.status='PASS';result.limits='Official window observation only; no compaction acceptance';
}catch(e){result.firstSeam=String(e?.message??e);}
finally{await r.record('result',result);await finish();}
if(result.status!=='PASS')process.exitCode=1;
