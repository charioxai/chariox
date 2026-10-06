// MP-08/MP-10/MP-11: protected fallback keeps the explicitly selected software codec.
import test from 'node:test';import assert from 'node:assert/strict';
import {PortableEncoder} from './kernel-browser-display.mjs';import {encodePng} from './kernel-browser-pixels.mjs';
test('MP-11 OpenH264 protected PNG fallback never silently chooses x264',async()=>{
 const before=process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER,software=process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE;
 process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER='libopenh264';process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE='1';
 const e=new PortableEncoder();
 try{const packet=await e.encode(encodePng(128,128,Buffer.alloc(128*128*4,255)),8000000,true,'avc1.420033');assert.equal(e.backend,'openh264');assert.equal(packet.key,true)}
 finally{await e.close();if(before===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER;else process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER=before;if(software===undefined)delete process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE;else process.env.CHARIOX_BROWSER_DISPLAY_SOFTWARE=software}
});
