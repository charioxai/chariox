// MP-08/MP-10/MP-11: private native-origin source bounds, not UI acceptance.
import test from 'node:test';import assert from 'node:assert/strict';import {nativePageRect} from './kernel-browser-page-geometry.mjs';
test('MP-08/MP-10/MP-11 native page geometry is independent of emulated1280x800',()=>{
 assert.deepEqual(nativePageRect({width:1280,height:800,frame_insets:[4,4,0,4]},{width:1272,height:709},1),{left:4,top:87,width:1272,height:709});
 assert.deepEqual(nativePageRect({width:2560,height:1600,frame_insets:[8,8,0,8]},{width:1272,height:709},2),{left:8,top:174,width:2544,height:1418});
});
test('MP-11 missing, malformed or emulated dimensions refuse native origin',()=>{
 const w={width:1280,height:800,frame_insets:[4,4,0,4]},v={width:1272,height:709};
 for(const [a,b,c] of [[w,{width:1280,height:800},1],[{...w,frame_insets:undefined},v,1],[{...w,frame_insets:[4,4,0,65]},v,1],[w,{width:1272,height:800},1],[w,{width:NaN,height:709},1],[w,v,3]])assert.throws(()=>nativePageRect(a,b,c),/origin/);
});
