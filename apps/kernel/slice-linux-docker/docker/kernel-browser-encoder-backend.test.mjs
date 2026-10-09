// MP-08/MP-10/MP-11: protected fallback keeps the explicitly selected software codec.
import test from 'node:test';import assert from 'node:assert/strict';
import {PortableEncoder} from './kernel-browser-display.mjs';import {encodePng} from './kernel-browser-pixels.mjs';
test('MP-10 actual SIMD conversion survives the browser encoder adapter',{skip:!process.env.CHARIOX_BROWSER_DISPLAY_LIBYUV},async()=>{
 const {BrowserEncoder}=await import('./kernel-browser-webcodecs.mjs');
 const e=new BrowserEncoder({},'target');
 try{
  const packet=await e.encodeStripes({width:128,height:128,format:'bgr0',length:128*128*4,pixels:Buffer.alloc(128*128*4)},8000000,true,'avc1.420033');
  assert.equal(packet.stripes.length,8);assert.equal(e.backend,'x264');assert.equal(e.converter,'libyuv');
  await e.encode(encodePng(128,128,Buffer.alloc(128*128*4,255)),8000000,true,'avc1.420033');
  assert.equal(e.converter,null,'a full-video fallback cannot retain the previous stripe converter receipt');
 }finally{await e.close()}
});
test('MP-11 OpenH264 protected PNG fallback never silently chooses x264',async()=>{
 const before=process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER,software=process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE;
 process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER='libopenh264';process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE='1';
 const e=new PortableEncoder();
 try{const packet=await e.encode(encodePng(128,128,Buffer.alloc(128*128*4,255)),8000000,true,'avc1.420033');assert.equal(e.backend,'openh264');assert.equal(packet.key,true)}
 finally{await e.close();if(before===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER;else process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER=before;if(software===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE;else process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE=software}
});

// MP-11: the browser encoder adapter must preserve packet ownership metadata.
test('MP-10 browser adapter forwards native ownership and worker reporting',async()=>{
 const {BrowserEncoder}=await import(process.env.MP_BROWSER_ENCODER_MODULE??'./kernel-browser-webcodecs.mjs');
 const discarded=[],handed=[],fallback={workers:2,discard:x=>discarded.push(x),handedOff:x=>handed.push(x),close:async()=>{}};
 const encoder=new BrowserEncoder({},'t',fallback),packet={packet:{name:'a'.repeat(32)+'.json'}};
 try{assert.equal(encoder.workers,2);encoder.discard(packet);encoder.handedOff(packet);assert.deepEqual(discarded,[packet]);assert.deepEqual(handed,[packet])}finally{await encoder.close()}
});
