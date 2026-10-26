// MP-08/MP-10/MP-11: fixture measurement contracts, never live acceptance.
import test from 'node:test';
import assert from 'node:assert/strict';
import {runInNewContext} from 'node:vm';
import {measureMotionTyping} from './drill-motion-input.mjs';

function page({refused=false,missingEcho=false,redrawing=false}={}){
 const inputs=[];let counter=0,now=100;
 const context={performance:{timeOrigin:0,now:()=>{if(missingEcho)now+=10001;return now}},
  setTimeout:callback=>callback(),requestAnimationFrame:callback=>callback(),
  mdPresentation:{},mdPresentations:[],mdStream:{binding:{device_scale_factor:2},presenter:{canvas:{width:2560}},
   async input(input){inputs.push(input);if(refused)throw Error('MP-11: input refused');if(input.kind==='text'&&!missingEcho){
    const shown={step:++counter,drawn_ms:110,presented_ms:125};context.mdPresentations.push(shown);
    Object.assign(context.mdPresentation,redrawing?{drawn_ms:130,presented_ms:undefined,step:undefined}:shown);
    if(redrawing)now+=10001;
   }}},
 };
 return {inputs,async evaluate(fn,value){return runInNewContext(`(${fn})(count)`,{...context,count:value})}};
}
test('MP-08/MP-10 motion typing records visible pixel echo through the admitted input adapter',async()=>{
 const fixture=page();const samples=await measureMotionTyping(fixture,2);
 assert.equal(samples.length,2);assert.deepEqual(Array.from(samples,s=>s.latency_ms),[25,25]);
 assert.deepEqual(fixture.inputs.map(input=>input.kind),['click','text','click','text']);
 assert.equal(fixture.inputs[0].x,1110,'DPR2 focus uses CSS coordinates');
});
test('MP-08/MP-10 motion typing keeps the first pixel echo when a newer frame is awaiting rAF',async()=>{
 const samples=await measureMotionTyping(page({redrawing:true}),1);
 assert.equal(samples[0].latency_ms,25);
});
test('MP-10/MP-11 missing echo and admission refusal cannot produce a passing typing sample',async()=>{
 await assert.rejects(measureMotionTyping(page({refused:true}),1),/input refused/);
 await assert.rejects(measureMotionTyping(page({missingEcho:true}),1),/pixel acknowledgement timeout/);
 for(const count of [0,21,NaN])await assert.rejects(measureMotionTyping(page(),count),/probe count/);
});
