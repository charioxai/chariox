// MP-08/MP-10/MP-11: native masking stays private and does not copy on motion.
import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,writeFileSync,readFileSync,readdirSync,rmSync} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {maskNativeRaster,maskPixels,displayMaskRegions} from './kernel-browser-pixels.mjs';
import {MotionEncoder} from './kernel-browser-motion.mjs';
import {PortableEncoder} from './kernel-browser-display.mjs';

test('MP-08/MP-10/MP-11 shared protected motion never reads or writes a Node raster',async()=>{
 const root=mkdtempSync(path.join(os.tmpdir(),'mp23-mask-'));
 const oldRoot=process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;
 process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=root;
 let copies=0,refs=1;
 const pixels=Buffer.alloc(128*128*4,210),file=path.join(root,'source');
 writeFileSync(file,pixels,{mode:0o600});
 const raw={width:128,height:128,length:pixels.length,format:'bgr0',serial:1,signature:'source',shared:{path:file,length:pixels.length},
  copyPixels(){copies++;return Buffer.from(pixels)},readRegion(x,y,w,h){const b=Buffer.alloc(w*h*4);for(let row=0;row<h;row++)pixels.copy(b,row*w*4,((y+row)*128+x)*4,((y+row)*128+x+w)*4);return b},retain(){refs++},release(){refs--}};
 const regions=[{x:40,y:40,width:30,height:30}];
 const encoder=new PortableEncoder();
 try{
  const masked=maskNativeRaster(raw,regions);raw.release();
  assert.equal(copies,0,'motion admission must not materialize the shared raster');
  assert.equal(masked.shared,undefined,'unmasked lease cannot masquerade as protected bytes');
  assert.deepEqual(masked[displayMaskRegions],regions);
  const motion=new MotionEncoder({sample:()=>({serial:1,raw:masked}),subscribe:()=>()=>{}},encoder,{bitrate:8000000,codec:'avc1.420033',stripes:true});
  await motion.active;const encoded=motion.take()?.encoded;assert.equal(motion.failure,undefined);await motion.close();
  assert.ok(encoded.stripes.length>0);assert.equal(copies,0,'encoder must consume native private masking');
  assert.deepEqual(readFileSync(file),pixels,'private codec masks must leave the capture lease unchanged');
  assert.equal(readdirSync(root).filter(name=>name.startsWith('encoder-')).length,0,'no Node handoff file');
  masked.release();assert.equal(refs,0);
 }finally{
  await encoder.close();
  if(oldRoot===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT;else process.env.CHARIOX_BROWSER_DISPLAY_PACKET_ROOT=oldRoot;
  rmSync(root,{recursive:true,force:true});
 }
});

test('MP-11 shared masked exact snapshots and cropped reads preserve padding and lease lifetime',()=>{
 const pixels=Buffer.alloc(128*128*4,99);let refs=1,copies=0;
 const raw={width:128,height:128,length:pixels.length,format:'bgr0',serial:1,shared:{path:'/private/lease',length:pixels.length},
  copyPixels(){copies++;return Buffer.from(pixels)},readRegion(x,y,w,h){const b=Buffer.alloc(w*h*4);for(let row=0;row<h;row++)pixels.copy(b,row*w*4,((y+row)*128+x)*4,((y+row)*128+x+w)*4);return b},retain(){refs++},release(){refs--}};
 const regions=[{x:40.5,y:40.5,width:30,height:30},{x:-100,y:0,width:20,height:20}];
 const masked=maskNativeRaster(raw,regions),expected=maskPixels({width:128,height:128,pixels:Buffer.from(pixels)},regions.map(r=>[r.x,r.y,r.width,r.height])).pixels;
 raw.release();assert.equal(copies,0);masked.retain();assert.equal(refs,2);
 const crop=masked.readRegion(32,32,48,48),wanted=Buffer.alloc(crop.length);
 for(let y=0;y<48;y++)expected.copy(wanted,y*48*4,((32+y)*128+32)*4,((32+y)*128+80)*4);
 assert.deepEqual(crop,wanted);assert.deepEqual(masked.pixels,expected);assert.equal(copies,1);
 masked.release();masked.release();assert.equal(refs,0);
});
