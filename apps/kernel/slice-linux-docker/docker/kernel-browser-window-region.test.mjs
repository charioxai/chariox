// MP-08/MP-10/MP-11: surface isolation and lease-bound crop coordinates.
import test from 'node:test';
import assert from 'node:assert/strict';
import {windowRegionGeometry,projectRegionMasks,NativeWindowRegionCapture} from './kernel-browser-window-region.mjs';
test('MP-08/MP-10 page and native Chrome crops share native pixels at both densities',()=>{
 for(const scale of [1,2]){
  const window={width:1280*scale,height:888*scale},content=800*scale;
  const chrome=windowRegionGeometry('browser_chrome',window,{left:0,top:0,width:1280*scale,height:88*scale},content,scale);
  assert.equal(chrome.css_width,1280);assert.equal(chrome.css_height,88);
  const page=windowRegionGeometry('page',window,{left:20*scale,top:108*scale,width:320*scale,height:240*scale},content,scale);
  assert.equal(page.pageLeft,20*scale);assert.equal(page.pageTop,20*scale);
  assert.equal(page.css_width,320);assert.equal(page.css_height,240);
  assert.deepEqual(projectRegionMasks([{x:22*scale,y:23*scale,width:10*scale,height:4*scale}],page.pageLeft,page.pageTop),
   [{x:2*scale,y:3*scale,width:10*scale,height:4*scale}]);
 }
});
test('MP-11 crops cannot spill across Chrome/page or name a malformed raster',()=>{
 const window={width:1280,height:888},rect={left:0,top:0,width:1280,height:88};
 for(const bad of [{...rect,height:90},{...rect,left:-1},{...rect,width:1282},{...rect,height:87}])
  assert.throws(()=>windowRegionGeometry('browser_chrome',window,bad,800,1),/region/);
 assert.throws(()=>windowRegionGeometry('page',window,rect,800,1),/boundary/);
 assert.throws(()=>windowRegionGeometry('root',window,rect,800,1),/region/);
 assert.throws(()=>windowRegionGeometry('browser_chrome',{...window,height:Infinity},rect,800,1),/region/);
});
test('MP-11 region coordinates and Chrome chords require a current kernel lease',()=>{
 const config={surface:'browser_chrome',window:{width:1280,height:888},rect:{left:0,top:0,width:1280,height:88},contentHeight:800,scale:1,
  isCurrent:()=>false,policy:{},allowed:()=>true};
 assert.throws(()=>new NativeWindowRegionCapture({...config,isCurrent:null}),/lease/);
 const source=new NativeWindowRegionCapture(config);
 assert.equal(source.point(1279,87),true);assert.equal(source.point(0,88),false);
 assert.equal(source.point(NaN,0),false);assert.equal(source.click(0,88),false);
 assert.equal(source.chord(0x6c,2),false);
});

test('MP-11 only a kernel-resolved exact X window may disambiguate owned equal-size windows',()=>{
 const w={id:42,width:1280,height:888},rect={left:0,top:0,width:1280,height:88};
 assert.equal(windowRegionGeometry('browser_chrome',w,rect,800,1).window_region.id,42);
 for(const id of [0,1,-1,NaN,2**32])assert.throws(()=>windowRegionGeometry('browser_chrome',{...w,id},rect,800,1),/window/);
});
