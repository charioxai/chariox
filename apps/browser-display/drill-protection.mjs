// MP-08/MP-10/MP-11: supplementary masking stress through physical input,
// production scoped relay frames and the real decoder/canvas. Not live acceptance.
import {verifyContinuousSettled} from './drill-settle.mjs';
export async function stressProtection({page,pair,pause,resource,repetitions}) {
 if(!Number.isSafeInteger(repetitions)||repetitions<1||repetitions>1000)throw Error('MP-11: protection repetition bound');
 const runs=[];
 for(let iteration=1;iteration<=repetitions;iteration++) {
  await resource();
  const before=await page.evaluate(()=>({frames:mdProtection.frames,violations:mdProtection.violations}));
  await page.evaluate(()=>mdStream.start());
  await page.evaluate(()=>mdStream.input({kind:'click',x:60,y:88}));
  await pause(350);
  const motion=await pair(`mask-${iteration}-motion`);
  await page.evaluate(()=>mdStream.input({kind:'click',x:60,y:88}));
  const settled=await verifyContinuousSettled(()=>pair(`mask-${iteration}-settle`));
  await page.evaluate(()=>mdStream.stop());
  const recovery=await page.evaluate(async()=>{
   // MP-08/MP-10 (491): a pushed stream recovers by an ACKed key request.
   const previous=mdStream.presenter.sequence;if(mdStream.push)mdStream.requestKey();else mdStream.presenter.sequence=0;
   const deadline=performance.now()+10000;let frame;
   const isIndependent=frame=>frame.kind==='png'||frame.kind==='video'&&frame.key||frame.kind==='stripes'&&frame.stripes.length===8&&frame.stripes.every(r=>r.key);
   while(!(frame=await mdStream.next())||mdStream.push&&!isIndependent(frame)){if(performance.now()>deadline)throw Error('MP-11: stress reference recovery timeout');await new Promise(r=>setTimeout(r,4));}
   const independent=isIndependent(frame);
   if(!independent)throw Error('MP-11: stress recovery reused a lost reference');
   return {previous,sequence:frame.sequence,kind:frame.kind,independent};
  });
  const recovered=await pair(`mask-${iteration}-recovery`);
  const after=await page.evaluate(()=>({frames:mdProtection.frames,violations:mdProtection.violations}));
  runs.push({iteration,motion,settled:settled.fidelity,recovery,recovered,presentations:after.frames-before.frames,violations:after.violations-before.violations});
 }
 return {item:'MP-08/MP-10/MP-11',repetitions,runs};
}
