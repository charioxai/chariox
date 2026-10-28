// MP-08/MP-10/MP-11: a loading background page cannot park native New Tab admission.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
import {KernelBrowserHost} from './kernel-browser-host.mjs';
import {InputArbiter} from './kernel-browser-chrome-window.mjs';
for(const pendingActive of [false,true])test('MP-08/MP-10/MP-11 newly active native tab gets its own document proof without querying a loading background renderer; pending active='+pendingActive,async()=>{
 const calls=[],viewport={css_width:1280,css_height:800,device_scale_factor:1,desktop_pixel_width:1280,desktop_pixel_height:800};
 const connection={isOpen:()=>true,send:async(method,args,session)=>{
  calls.push({method,session});
  if(method==='Target.getTargets')return {targetInfos:['old','new'].map(targetId=>({type:'page',targetId,url:'https://www.wikipedia.org/',title:'Wikipedia'}))};
  if(method==='Emulation.setDeviceMetricsOverride')return {};
  if(method==='Target.getTargetInfo')return {targetInfo:{url:'https://www.wikipedia.org/',title:'Wikipedia'}};
  if(method==='Page.getFrameTree'){if(session==='old-session')throw new Error('loading background renderer queried');return {frameTree:{frame:{id:'new-frame',loaderId:'new-doc'}}};}
  assert.fail('unexpected method '+method);
 }};
 const browser=new BrowserCdpClient({connectionFactory:async()=>connection});browser.connection=connection;browser.ensureConnection=async()=>connection;browser.ensureTargetSession=async(_c,target)=>target+'-session';browser.readFocus=async()=>false;
 for(const target of ['old','new'])browser.sessionsByTarget.set(target,target+'-session');
 browser.documentIdsByTarget.set('old','old-doc');browser.viewportByTarget.set('old',{key:JSON.stringify({width:1280,height:800,deviceScaleFactor:1,mobile:false,screenWidth:1280,screenHeight:800})});
 if(pendingActive){browser.documentIdsByTarget.set('new','last-committed');browser.viewportByTarget.set('new',browser.viewportByTarget.get('old'));}
 const host={browser,chromium:{desktop:{binding:()=>({})}},generation:1,layouts:new Map(),layoutRevisions:new Map(),observations:0,observed:0,activeTarget:'old',tabs:new Map([['old-tab',{tab_id:'old-tab',target_id:'old',document_id:'old-doc'}]]),streams:new Map(),tabGroups:{active:async()=> 'new'},viewport:()=>viewport,tabState(){return {tabs:[...this.tabs.values()]};},save:async()=>{},retireNativeForeground:async()=>{}};
 host.reconcile=options=>KernelBrowserHost.prototype.reconcile.call(host,options);
 let documentProofs=0;host.displayTarget=command=>{documentProofs++;return KernelBrowserHost.prototype.displayTarget.call(host,command);};
 const arbiter=new InputArbiter(host,{current:()=>({tabbed:{window_generation:2,isCurrent:()=>true}})});
 const admitted=await arbiter.resolveDisplayInput({window_generation:2});assert.equal(admitted.document_id,'new-doc');assert.equal(host.tabs.get(admitted.tab_id).target_id,'new');assert.equal(documentProofs,1);
 assert(calls.some(c=>c.method==='Page.getFrameTree'&&c.session==='new-session'),'the new target has a fresh document proof');assert(!calls.some(c=>c.method==='Page.getFrameTree'&&c.session==='old-session'));
 assert.equal(browser.currentPageFrame('old'),null,'advisory metadata did not grant a background document owner');
});
