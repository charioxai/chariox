import test from 'node:test';
import assert from 'node:assert/strict';
import {fitNativeViewport} from './kernel-browser-native-viewport.mjs';

function windowFixture({width=1280,height=800,hostScale=1,insetX=8,insetY=91}={}){
 let bounds,sets=0;
 return {width,height,hostScale,windowId:4,sessionId:'owned',wait:async()=>{},
  connection:{async send(method,params){
   if(method==='Browser.setWindowBounds'){bounds={...params.bounds};sets++;}
   if(method==='Browser.getWindowBounds')return {bounds};
   return {};
  }},measure:async()=>({width:(bounds.width-insetX)*hostScale,height:(bounds.height-insetY)*hostScale}),
  get sets(){return sets;}};
}
for(const scale of [1,2])test(`MP-08/MP-10/MP-11 fit decorated native viewport at DPR${scale}`,async()=>{
 const fixture=windowFixture({width:1280*scale,height:800*scale});
 await fitNativeViewport(fixture);assert.equal(fixture.sets,2);
});
test('MP-08/MP-10/MP-11 refuse a viewport that never reaches the requested geometry',async()=>{
 const fixture=windowFixture();fixture.measure=async()=>({width:1,height:1});
 await assert.rejects(fitNativeViewport(fixture),/native viewport geometry/);
 assert.ok(fixture.sets<=4);
});
test('MP-08/MP-10/MP-11 fit host DPR2 from physical reference pixels',async()=>{
 const fixture=windowFixture({width:2560,height:1600,hostScale:2});
 await fitNativeViewport(fixture);assert.equal(fixture.sets,2);
});
test('MP-08/MP-10/MP-11 refuse unsettled window bounds before measuring pixels',async()=>{
 const fixture=windowFixture();let measured=false;
 fixture.connection.send=async()=>({bounds:{width:1,height:1}});
 fixture.measure=async()=>{measured=true;return {width:1280,height:800};};
 await assert.rejects(fitNativeViewport(fixture),/geometry unsettled/);assert.equal(measured,false);
});
test('MP-08/MP-10/MP-11 bounded correction refuses repeated small mismatches',async()=>{
 const fixture=windowFixture();fixture.measure=async()=>({width:1272,height:796});
 await assert.rejects(fitNativeViewport(fixture),/geometry differed/);assert.equal(fixture.sets,4);
});
