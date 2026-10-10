import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayCapture, changedClip } from './kernel-browser-display-capture.mjs';
import { encodePng, displayMaskRegions } from './kernel-browser-pixels.mjs';
import { captureProtectedDisplay } from './kernel-browser-region-protection.mjs';
import { DisplayStream, PortableEncoder } from './kernel-browser-display.mjs';
import { randomBytes } from 'node:crypto';

test('MP-11: protected crop over patch budget keeps all full-raster masks through real encoder',async()=>{
 const tab={tab_id:'t',document_id:'d',input_epoch:0},policy={values:[]};
 const masks=[{x:900,y:200,width:24,height:24},{x:40,y:60,width:32,height:32}];
 const pixels=Buffer.alloc(1280*800*4,255);
 // The product screenshot contract already masks these pixels before encode.
 // Keep the mocked raster valid so this test reaches full-mask propagation.
 const redact=()=>{for(const r of masks)for(let y=r.y;y<r.y+r.height;y++)pixels.fill(Buffer.from([0,0,0,255]),(y*1280+r.x)*4,(y*1280+r.x+r.width)*4)};
 redact();
 const host={generation:1,scales:new Map([['t',1]]),screenshot:async()=>({...tab,generation:1,protected_regions:masks,data_base64:encodePng(1280,800,pixels)})};
 const capture=new DisplayCapture(clip=>captureProtectedDisplay(host,tab,clip),1);
 const encoder=new PortableEncoder();let encodedMasks;
 const encode=encoder.encode.bind(encoder);encoder.encode=(...args)=>{encodedMasks=args[4];return encode(...args)};
 let now=0;
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',document_id:'d',device_scale_factor:1,bitrate:128000,codec:'png',dependencies:true},{encoder,now:()=>now,wait:async ms=>{now+=ms}});
 try{
  const initial=await capture.next(tab,policy,false);
  await stream.frame(initial,'d',0);
  const noise=randomBytes(112*112*4);
  for(let row=0;row<112;row++)noise.copy(pixels,((184+row)*1280+880)*4,row*112*4,(row+1)*112*4);
  redact();
  tab.input_epoch++;
  const merged=await capture.next(tab,policy,true);
  assert(merged.dirty_clip,'must exercise crop merge');
  stream.codec='avc1.420033';
  const frame=await stream.frame(merged,'d',stream.sequence);
  assert(encodedMasks,'patch must exceed budget and reach the real full-frame encoder');
  assert.deepEqual(encodedMasks,masks,'include protected fields outside crop');
  assert.equal(encoder.failure,null);
  assert(['video','png'].includes(frame.kind),'decoded guard may choose protected exact fallback');
 }finally{await stream.close()}
});

test('MP-11: real full-frame codec ignores offscreen masks and guards partial intersections',async()=>{
 const encoder=new PortableEncoder(),pixels=Buffer.alloc(128*128*4,255);
 const masks=[{x:-100,y:0,width:20,height:20},{x:140,y:0,width:20,height:20},
  {x:0,y:-100,width:20,height:20},{x:0,y:140,width:20,height:20},
  {x:-10,y:32,width:42,height:32}];
 for(let y=32;y<64;y++)pixels.fill(Buffer.from([0,0,0,255]),y*128*4,(y*128+32)*4);
 try{
  const frame=await encoder.encode(encodePng(128,128,pixels),8000000,true,'avc1.420033',masks);
  assert.equal(encoder.failure,null);assert(frame.dropped||frame.key);
  pixels[(40*128+10)*4]=255;
  await assert.rejects(encoder.encode(encodePng(128,128,pixels),8000000,true,'avc1.420033',masks),/encoder unavailable/);
 }finally{await encoder.close()}
});
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
test('MD-DISPLAY repeated verified full PNG skips decode but never skips changed pixels',async()=>{
 let time=0,changed=false;const timings=[];
 const tab={tab_id:'t',document_id:'d'},policy={values:[]};
 const capture=new DisplayCapture(async clip=>{const w=clip?160:1280,h=clip?100:800;return {...tab,generation:1,data_base64:encodePng(w,h,Buffer.alloc(w*h*4,changed?127:255))}},1,name=>timings.push(name),()=>time);
 await capture.next(tab,policy,false,true);timings.length=0;time=300;
 const same=await capture.next(tab,policy,true,true);assert.equal(same.pixels.pixels[0],255);
 assert.ok(!timings.includes('full_source_decode'));
 changed=true;timings.length=0;time=600;
 const changedFrame=await capture.next(tab,policy,true,true);assert.equal(changedFrame.pixels.pixels[0],127);
 assert.ok(timings.includes('full_source_decode'));
});
test('MD-DISPLAY changing whole viewport uses CSS motion resolution; idle returns to native pixels',async()=>{
 let changing=false,time=0;
 const tab={tab_id:'t',document_id:'d'},policy={values:[]},calls=[];
 const capture=new DisplayCapture(async clip=>{calls.push(clip?.scale??1);const width=clip?Math.round(1280*clip.scale*2):2560,height=clip?Math.round(800*clip.scale*2):1600;return {...tab,generation:1,width,height,data_base64:encodePng(width,height,Buffer.alloc(width*height*4,changing?127:255))}},2,()=>{},()=>time);
 const motion={x:0,y:0,width:1280,height:800,scale:.5};
 await capture.next(tab,policy,false,true,motion);changing=true;
 const moving=await capture.next(tab,policy,true,true,motion);
 assert.equal(moving.motion,true);assert.equal(moving.width,1280);assert.equal(calls.at(-1),.5);
 time=351;const idle=await capture.next(tab,policy,true,true,motion);assert.equal(idle.motion,undefined);assert.equal(idle.pixels.width,2560);
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
test('MD-DISPLAY admitted scroll uses motion even when thumbnail misses repeating text',async()=>{
 const tab={tab_id:'t',document_id:'d'},policy={values:[]};let time=0;
 const capture=new DisplayCapture(async clip=>{const width=clip?Math.round(1280*clip.scale*2):2560,height=clip?Math.round(800*clip.scale*2):1600;return {...tab,generation:1,width,height,data_base64:encodePng(width,height,Buffer.alloc(width*height*4,255))}},2,()=>{},()=>time);
 const motion={x:0,y:240,width:1280,height:800,scale:.5};
 await capture.next(tab,policy,false,true,motion);
 const moving=await capture.next(tab,policy,true,true,motion,true);assert.equal(moving.motion,true);assert.equal(moving.width,1280);
 time=400;const idle=await capture.next(tab,policy,true,true,motion,false);assert.equal(idle.motion,undefined);assert.equal(idle.pixels.width,2560);
});

test('MD-DISPLAY compositor idle reuse requires verified native epoch and policy',async()=>{
 let clock=0;const data=encodePng(1280,800,Buffer.alloc(1280*800*4,255));
 const tab={tab_id:'t',document_id:'d',input_epoch:1},policy={values:[]};
 const capture=new DisplayCapture(async()=>({...tab,data_base64:data}),1,()=>{},()=>clock);
 await capture.next(tab,policy,false,true);
 assert(capture.verified(tab,policy,1));assert.equal(capture.verified(tab,policy,2),null);
 assert.equal(capture.verified({...tab,document_id:'new'},policy,1),null);
 assert.equal(capture.verified(tab,{values:[]},1),null);
 clock=251;assert.equal(capture.verified(tab,policy,1),null);
});
