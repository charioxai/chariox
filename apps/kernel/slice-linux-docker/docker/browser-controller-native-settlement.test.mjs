import test from 'node:test';import assert from 'node:assert/strict';import {BrowserCdpClient} from './browser-controller-cdp.mjs';
test('MP-08/MP-09/MP-10/MP-11: maximize acknowledgment waits for the native content resize',async()=>{
 let reads=0;const sent=[];const browser=new BrowserCdpClient({applyCanonicalDisplay:async()=>{}});browser.canonical={desktop_pixel_width:2560,desktop_pixel_height:1600};
 const connection={async send(method,params){sent.push({method,params});if(method==='Browser.getWindowForTarget')return{windowId:1,bounds:{left:0,top:0,width:2560,height:1600,windowState:'maximized'}};if(method==='Page.getLayoutMetrics')return{cssVisualViewport:{zoom:1,scale:1}};if(method==='Page.getFrameTree')return{frameTree:{frame:{id:'f'}}};if(method==='Page.createIsolatedWorld')return{executionContextId:1};if(method==='Runtime.evaluate')return{result:{value:++reads<3?{width:1050,height:693}:{width:2560,height:1513}}};return{}}};
 try{const applied=await browser.setPageMetrics(connection,'s',{width:1280,height:800,deviceScaleFactor:2,scale:2,mobile:false});assert.equal(applied.width,1280);assert.equal(applied.height,756);assert.ok(reads>=3);assert.deepEqual(sent.findLast(x=>x.method==='Emulation.setVisibleSize').params,{width:2560,height:1512})}finally{await browser.close()}
});
test('MP-08/MP-09/MP-10/MP-11: zoomed maximize waits for window-unit content and unknown zoom is refused',async()=>{
 for(const zoom of [1.25,undefined]){
  let reads=0;const browser=new BrowserCdpClient({applyCanonicalDisplay:async()=>{}});browser.canonical={desktop_pixel_width:1280,desktop_pixel_height:800};
  const connection={async send(method){if(method==='Browser.getWindowForTarget')return{windowId:1,bounds:{left:0,top:0,width:1280,height:800,windowState:'maximized'}};if(method==='Page.getLayoutMetrics')return{cssVisualViewport:{zoom,scale:1}};if(method==='Page.getFrameTree')return{frameTree:{frame:{id:'f'}}};if(method==='Page.createIsolatedWorld')return{executionContextId:1};if(method==='Runtime.evaluate')return{result:{value:++reads<3?{width:840,height:555}:{width:1024,height:570}}};return{}}};
  try{
   const applying=browser.setPageMetrics(connection,'s',{width:1280,height:800,deviceScaleFactor:1,scale:1,mobile:false});
   if(zoom===undefined)await assert.rejects(applying,error=>error.code==='viewport_apply_failed'&&/zoom/.test(error.message));
   else{const applied=await applying;assert.equal(applied.width,1280);assert.ok(applied.height>=711&&applied.height<=713);assert.ok(reads>=3);}
  }finally{await browser.close()}
 }
});
