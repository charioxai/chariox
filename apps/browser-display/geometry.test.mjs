// MP-08/MP-10/MP-11: the existing frame fields carry bounded 1080p geometry.
import test from 'node:test';
import assert from 'node:assert/strict';
import {DisplayStream} from '../kernel/slice-linux-docker/docker/kernel-browser-display.mjs';
import {execFileSync} from 'node:child_process';
test('MP-08/MP-10 motion packet uses its canonical 1080p viewport',async()=>{
 const stream=new DisplayStream({subscription_id:'s',tab_id:'t',codec:'avc1.420033',bitrate:8000000,device_scale_factor:1,css_width:1920,css_height:1080,dependencies:true},{encoder:{encode:async()=>({key:true,data_base64:'YQ=='}),close:async()=>{}},now:()=>0,wait:async()=>{}});
 try{const frame=await stream.frame({generation:1,motion:true,width:1920,height:1080,data_base64:'frame'},'document',0);assert.deepEqual([frame.width,frame.height,frame.css_width,frame.css_height],[1920,1080,1920,1080]);}finally{await stream.close()}
});
test('MP-11 1080p capture keeps an opaque whole frame under unknown protection',()=>{
 const script="import {wholeFrameMask,decodePng} from './apps/kernel/slice-linux-docker/docker/kernel-browser-pixels.mjs';const p=decodePng(wholeFrameMask());if(p.width!==1920||p.height!==1080||p.pixels.some((v,i)=>v!==(i%4===3?255:0)))throw Error('whole-frame geometry/mask');";
 execFileSync(process.execPath,['--input-type=module','-e',script],{env:{...process.env,CHARIOX_BROWSER_DISPLAY_GEOMETRY:'1920x1080'}});
});
test('MP-11 protected regions at 1080p remain in native DPR1 coordinates',()=>{
 const script="import {captureRegionMasks} from './apps/kernel/slice-linux-docker/docker/kernel-browser-region-protection.mjs';const c={send:async m=>m==='DOM.getDocument'?{root:{nodeId:1}}:m==='DOM.querySelectorAll'?{nodeIds:[2]}:{model:{border:[1700,900,1800,900,1800,950,1700,950]}}};const m=await captureRegionMasks(c,'s');const r=await m.afterCapture({width:1920,height:1080});if(JSON.stringify(r)!==JSON.stringify([{x:1700,y:900,width:100,height:50}]))throw Error('protection scaled outside field');";
 execFileSync(process.execPath,['--input-type=module','-e',script],{env:{...process.env,CHARIOX_BROWSER_DISPLAY_GEOMETRY:'1920x1080'}});
});
