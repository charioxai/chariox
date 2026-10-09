// MP-08/MP-10/MP-11: trusted real-browser wheel events at a bounded cadence.
// CDP drives the built UI; no kernel request or synthetic DOM event is injected.
import {setTimeout as delay} from 'node:timers/promises';
export async function driveHostedWheel(cdp,bounds,{durationMs=10000,hz=60}={}){
 const pending=new Set(),started=performance.now();let sent=0,dropped=0,failure;
 const x=bounds.x+bounds.width/2,y=bounds.y+bounds.height/2;
 for(let tick=0;performance.now()-started<durationMs;tick++){
  if(pending.size<4){
   const deltaY=Math.floor((performance.now()-started)/1000)%2===0?120:-120;
   const job=cdp.send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY});
   sent++;pending.add(job);job.catch(()=>failure=Error('MP-10: trusted wheel dispatch failed')).finally(()=>pending.delete(job));
  }else dropped++;
  await delay(Math.max(0,started+(tick+1)*1000/hz-performance.now()));
 }
 await Promise.allSettled([...pending]);if(failure)throw failure;
 return {sent,dropped,target_hz:hz,max_in_flight:4,duration_ms:performance.now()-started};
}
