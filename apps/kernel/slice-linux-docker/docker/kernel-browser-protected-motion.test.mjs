// MP-08/MP-10/MP-11: protected motion and bounded exact fallback regressions.
import {test} from 'node:test';import assert from 'node:assert/strict';
import {MotionEncoder} from './kernel-browser-motion.mjs';import {displayMaskRegions} from './kernel-browser-pixels.mjs';
const source=raw=>({sample:()=>({serial:1,raw}),subscribe:()=>()=>{}});
test('MP-11: protected dense native motion retains the stripe path',async()=>{
 const calls=[],raw={width:2560,height:1600,damage:[0,0,2560,1600],motion_height:1600,nativeEncode(){},[displayMaskRegions]:[{x:1800,y:400,width:300,height:160}]};
 const encoder={encode:async()=>{calls.push('whole');return {key:true}},encodeStripes:async()=>{calls.push('rows');return {stripes:[]}}};
 const motion=new MotionEncoder(source(raw),encoder,{bitrate:8000000,codec:'avc1.420033',stripes:true});
 try{await motion.active;assert.deepEqual(calls,['rows']);assert.equal(motion.failure,undefined)}finally{await motion.close()}
});
test('MP-11: safe PNG fallback above the video budget can settle; oversize stays bounded',async()=>{
 for(const bytes of [1100000,4*1024*1024+1]){
  const raw={width:2560,height:1600,nativeEncode(){},nativeExact:async()=>({data_base64:'a'.repeat(bytes)})};
  const motion=new MotionEncoder(source(raw),{encode:async()=>({dropped:true})},{bitrate:8000000,codec:'avc1.420033'});
  try{await motion.active;if(bytes<=4*1024*1024){assert.equal(motion.failure,undefined);assert.equal(motion.take().force_lossless,true)}else{assert(motion.failure);assert.equal(motion.frames.length,0)}}finally{await motion.close()}
 }
});
