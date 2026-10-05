import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayCapture, changedClip } from './kernel-browser-display-capture.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';
test('MD-DISPLAY crop pixels stay native; unchanged preview verifies missed fine detail', async () => {
  let pixels=Buffer.alloc(1280*800*4,255), thumbnail=Buffer.alloc(160*100*4,255);
  const calls=[], tab={tab_id:'t',document_id:'d'}, policy={};
  const capture = new DisplayCapture(async clip => {
    calls.push(clip?.scale < 1?'preview':clip?'crop':'full');
    const width=clip?(clip.scale<1?160:clip.width):1280, height=clip?(clip.scale<1?100:clip.height):800;
    const data=clip?.scale<1?thumbnail:Buffer.alloc(width*height*4);
    if(!clip||clip.scale===1) for(let row=0;row<height;row++)pixels.copy(data,row*width*4,((clip?.y??0)+row)*1280*4+(clip?.x??0)*4,((clip?.y??0)+row)*1280*4+(clip?.x??0)*4+width*4);
    return {...tab,generation:1,data_base64:encodePng(width,height,data)};
  },1);
  await capture.next(tab,policy,false);calls.length=0;
  pixels[(104*1280+104)*4]=0;thumbnail[(13*160+13)*4]=0;
  const patch=await capture.next(tab,policy,true);
  assert.deepEqual(calls,['preview','crop']);assert.deepEqual(patch.pixels.pixels,pixels);
  // A one-pixel change not represented in the thumbnail must be read back.
  pixels[(700*1280+1000)*4]=0;calls.length=0;
  const verified=await capture.next(tab,policy,true);
  assert.deepEqual(calls,['full']);assert.deepEqual(verified.pixels.pixels,pixels);
  calls.length=0;await capture.next(tab,{},true);assert.deepEqual(calls,['preview','full'],'policy replacement fences cached pixels');
  calls.length=0;await capture.next(tab,policy,false);assert.deepEqual(calls,['preview','full'],'lost credit base fences cache');
});
test('MD-DISPLAY large preview damage falls back; wrong document is rejected', async () => {
 const before={width:160,height:100,pixels:Buffer.alloc(160*100*4)}, after={...before,pixels:Buffer.alloc(160*100*4,255)};
 assert.equal(changedClip(before,after,.125),null);
 const capture=new DisplayCapture(async()=>({tab_id:'t',document_id:'other',data_base64:''}),1);
 await assert.rejects(capture.next({tab_id:'t',document_id:'d'},{},false),/binding changed/);
 assert.equal(capture.previous,null);
});
test('MD-DISPLAY verified idle pixels avoid repeated full readback until deadline or input',async()=>{
 let time=0,calls=0;
 const pixels=Buffer.alloc(1280*800*4,255),tab={tab_id:'t',document_id:'d',input_epoch:0},policy={values:[]};
 const capture=new DisplayCapture(async clip=>{if(!clip)calls++;const width=clip?160:1280,height=clip?100:800;return {...tab,generation:1,data_base64:encodePng(width,height,clip?Buffer.alloc(width*height*4,255):pixels)}},1,()=>{},()=>time);
 await capture.next(tab,policy,false);assert.equal(calls,1);
 await capture.next(tab,policy,true);assert.equal(calls,1);
 time=251;await capture.next(tab,policy,true);assert.equal(calls,2);
 await capture.next({...tab,input_epoch:1},policy,true);assert.equal(calls,3);
});
test('MD-DISPLAY changing whole viewport uses CSS motion resolution; idle returns to native pixels',async()=>{
 let changing=false,time=0;
 const tab={tab_id:'t',document_id:'d'},policy={values:[]},calls=[];
 const capture=new DisplayCapture(async clip=>{calls.push(clip?.scale??1);const width=clip?Math.round(1280*clip.scale*2):2560,height=clip?Math.round(800*clip.scale*2):1600;return {...tab,generation:1,width,height,data_base64:encodePng(width,height,Buffer.alloc(width*height*4,changing?127:255))}},2,()=>{},()=>time);
 const motion={x:0,y:0,width:1280,height:800,scale:.5};
 await capture.next(tab,policy,false,true,motion);changing=true;
 const moving=await capture.next(tab,policy,true,true,motion);
 assert.equal(moving.motion,true);assert.equal(moving.width,1280);assert.equal(calls.at(-1),.5);
 time=251;const idle=await capture.next(tab,policy,true,true,motion);assert.equal(idle.motion,undefined);assert.equal(idle.pixels.width,2560);
});
test('MD-DISPLAY forced verification of small input damage never enters motion mode',async()=>{
 let changed=false;
 const tab={tab_id:'t',document_id:'d',input_epoch:0},policy={values:[]};
 const capture=new DisplayCapture(async clip=>{const width=clip?Math.round(1280*clip.scale):1280,height=clip?Math.round(800*clip.scale):800;const pixels=Buffer.alloc(width*height*4,255);if(changed)pixels[0]=0;return {...tab,generation:1,width,height,data_base64:encodePng(width,height,pixels)}},1);
 const motion={x:0,y:0,width:1280,height:800,scale:1};
 await capture.next(tab,policy,false,true,motion);changed=true;
 const source=await capture.next({...tab,input_epoch:1},policy,true,true,motion);
 assert.equal(source.motion,undefined);assert.equal(source.pixels.width,1280);
});
test('MD-DISPLAY private prefetch is single-flight and fenced by input, policy and close',async()=>{
 let time=0,calls=0,finish;
 const tab={tab_id:'t',document_id:'d',input_epoch:0},policy={values:[]},clip={x:0,y:0,width:1280,height:800,scale:.5};
 const source={...tab,generation:1,width:1280,height:800,data_base64:'opaque-protected-motion'};
 const capture=new DisplayCapture(async()=>{calls++;return source},2,()=>{},()=>time);
 Object.assign(capture,{document:'d',policy,motion:true});
 capture.prefetch(tab,policy,clip);capture.prefetch(tab,policy,clip);
 await Promise.allSettled([...capture.tasks]);assert.equal(calls,1);
 assert.equal((await capture.next(tab,policy,true,true,clip)).motion,true);assert.equal(calls,1);
 capture.prefetch(tab,policy,clip);await Promise.allSettled([...capture.tasks]);
 await capture.next({...tab,input_epoch:1},policy,true,true,clip);assert.equal(calls,3,'input invalidates the captured epoch');
 capture.capture=()=>new Promise(resolve=>{finish=resolve});capture.prefetch(tab,policy,clip);
 let closed=false;const closing=capture.close().then(()=>closed=true);
 await Promise.resolve();assert.equal(closed,false);finish(source);await closing;
 assert.equal(capture.prefetched,null);assert.equal(capture.tasks.size,0);
 // Policy invalidation while a readback is pending can never publish old bytes.
 Object.assign(capture,{document:'d',policy,motion:true});capture.prefetch(tab,policy,clip);capture.invalidate();
 finish(source);await Promise.allSettled([...capture.tasks]);assert.equal(capture.prefetched,null);
});
