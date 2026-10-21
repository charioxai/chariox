// MP-08/MP-11: unchanged-capture fence regression.
import test from 'node:test';
import assert from 'node:assert/strict';
import {ProtectionGate,activeBrowserPage,measureBrowserProtection,awaitPresented} from './browser-protection-regions.mjs';

for (const result of [true,false,'failure']) {
  test(`MP-08/MP-11: presentation session settles on ${result}`, async () => {
    const calls=[];
    const connection={async send(method,params,sessionId) {
      calls.push({method,params,sessionId});
      if(method==='Target.attachToTarget')return {sessionId:'owned-session'};
      if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame'}}};
      if(method==='Page.createIsolatedWorld')return {executionContextId:7};
      if(method==='Runtime.evaluate') {
        assert.equal(sessionId,'owned-session');assert.equal(params.awaitPromise,true);
        assert.match(params.expression,/setTimeout\(\(\) => resolve\(false\), 500\)/);
        if(result==='failure')throw Error('MP-11 synthetic CDP failure');
        return {result:{value:result}};
      }
      return {};
    }};
    const browser={async ensureConnection(){return connection}};
    if(result===true)await awaitPresented(browser,[{target_id:'visible-page'}]);
    else await assert.rejects(awaitPresented(browser,[{target_id:'visible-page'}]),result===false?/did not present/:/synthetic CDP failure/);
    assert.deepEqual(calls[0],{method:'Target.attachToTarget',params:{targetId:'visible-page',flatten:true},sessionId:undefined});
    assert.deepEqual(calls.at(-1),{method:'Target.detachFromTarget',params:{sessionId:'owned-session'},sessionId:undefined});
  });
}
test('stream gate adopts only stable protection and releases only verified frames', async () => {
  const sequence = ['A', 'A', 'A', 'B', 'B', null, 'B'];
  let clock = 0;
  const gate = new ProtectionGate(async () => { const next = sequence.shift(); if (next === null) throw new Error('unbound'); return { pages: [next] }; });
  const steps = [];
  for (let i = 0; i < 7; i++) steps.push({ ...(await gate.step(() => ++clock)), serial: gate.protectionSerial });
  assert.deepEqual(steps.map(({ verified, changed, serial }) => [verified, changed, serial]), [
    [null, false, 0], // first sight: withheld
    [null, true, 1],  // stable across a presented frame: adopt
    [1, false, 1],    // frames captured with 1 before this step are released
    [null, true, 0],  // changed layout: drop frames, withhold
    [null, true, 2],  // new layout stable: adopt
    [null, true, 0],  // unbound measurement: withhold
    [null, false, 0], // one sighting is not enough
  ]);
  assert.equal(gate.protection, null);
});

test('MP-08/MP-10/MP-11 no filled browser field needs no renderer geometry or presentation fence',async()=>{
 const browser={fillTargets:new Map(),ensureConnection:async()=>{assert.fail('no filled field requires CDP work')}};
 const policy={unknown:false,values:['public-registration-only'],targets:[{kind:'browser',target_id:'ordinary',node_ref:'backend:1',document_id:'doc'}]};
 assert.deepEqual(await measureBrowserProtection(browser,policy),{pages:[]});
 await awaitPresented(browser,[]);
 browser.fillTargets.set('pending',{kind:'browser',target_id:'ordinary',node_ref:'backend:1',document_id:'doc',value_hash:'public-hash',pending:true});
 await assert.rejects(measureBrowserProtection(browser,policy),/no filled field requires CDP work/,'pending fills retain the capture proof');
 await assert.rejects(measureBrowserProtection(browser,{...policy,unknown:true}),/fill policy unavailable/);
});

// MP-11: generic password/search policies no longer inspect page fields.
test('MP-11 no recorded fill targets means no generic password or echo scan', async () => {
  const {measurePageProtection}=await import('./browser-protection-regions.mjs');
  const calls=[];const connection={async send(method) {
    calls.push(method);
    if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:'l',url:'https://example.test/'}}};
    if(method==='Page.createIsolatedWorld')return {executionContextId:1};
    if(method==='Runtime.evaluate')return {result:{value:{visible:true,dpr:1,width:1280,height:800}}};
    if(method==='Page.getLayoutMetrics')return {cssVisualViewport:{zoom:1}};
    assert.fail('unregistered fields cannot trigger an inspection');
  }};
  assert.deepEqual((await measurePageProtection(connection,'s','t',{targets:[],values:['registered']})).regions,[]);
  assert(calls.every(m=>!m.startsWith('DOM.')));
});

const placementConnection=(url='https://example.test/a')=>({async send(method,params) {
  if(method==='Target.getTargets')return {targetInfos:[{targetId:'t',type:'page',url}]};
  if(method==='Target.getTargetInfo')return {targetInfo:{targetId:params.targetId,url}};
  if(method==='Browser.getWindowForTarget')return {windowId:3,bounds:{left:40,top:30,width:900,height:700,windowState:'normal'}};
  if(method==='Target.attachToTarget')return {sessionId:'s'};
  if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:'l',url}}};
  if(method==='Page.createIsolatedWorld')return {executionContextId:1};
  if(method==='Runtime.evaluate')return {result:{value:{visible:true,dpr:1,width:892,height:553}}};
  if(method==='Page.getLayoutMetrics')return {cssVisualViewport:{zoom:1}};
  return {};
}});
test('MP-11 every measured page carries the extension native Tab size that places it on X11, none when unavailable',async()=>{
  const browser={browserGeneration:1,fillTargets:new Map([['k',{target_id:'other',browser_generation:1}]]),async ensureConnection(){return placementConnection()},
    nativeViewport:async target=>({target_id:target,width:892,height:553})};
  const policy={unknown:false,targets:[],values:[]};
  assert.deepEqual((await measureBrowserProtection(browser,policy)).pages.map(p=>[p.window,p.native,p.viewport]),[[[40,30,900,700],[892,553],[892,553]]]);
  delete browser.nativeViewport;
  assert.equal((await measureBrowserProtection(browser,policy)).pages[0].native,null);
  browser.nativeViewport=async()=>null;
  assert.equal((await measureBrowserProtection(browser,policy)).pages[0].native,null);
});
test('MP-11 the hit-test page is the active Tab from CDP and the extension, never an accessibility tree',async()=>{
  const browser={async ensureConnection(){return placementConnection()},nativeViewport:async target=>{assert.equal(target,undefined);return {target_id:'t',width:892,height:553};}};
  assert.deepEqual(await activeBrowserPage(browser),{url:'https://example.test/a',window:[40,30,900,700],native:[892,553],overlay:false});
  assert.equal(await activeBrowserPage({...browser,nativeViewport:async()=>null}),null);
  assert.equal(await activeBrowserPage({ensureConnection:browser.ensureConnection}),null);
});

// Chrome 155 draws the omnibox list in the page's own X client: the persistent browser_ui WebUI is the only proof.
const uiConnection=(visibility,{fail=false}={})=>{const base=placementConnection(),sent=[];
  return {sent,async send(method,params,session){sent.push(method);
    if(method==='Target.getTargets'){const {targetInfos}=await base.send(method);
      return {targetInfos:[...targetInfos,...Object.keys(visibility).map(id=>({targetId:id,type:'browser_ui',url:`chrome://omnibox-popup.top-chrome/${id}`}))]};}
    if(method==='Target.attachToTarget'&&visibility[params.targetId]!==undefined)return {sessionId:params.targetId};
    if(method==='Runtime.evaluate'&&visibility[session]!==undefined){if(fail)throw new Error('gone');return {result:{value:visibility[session]}};}
    if(method==='Target.detachFromTarget')return {};
    return base.send(method,params,session);}};};
const uiBrowser=connection=>({async ensureConnection(){return connection},nativeViewport:async()=>({target_id:'t',width:892,height:553})});
test('MP-11 a visible in-window browser WebUI (omnibox list) over the active Tab is reported as an overlay',async()=>{
  assert.equal((await activeBrowserPage(uiBrowser(uiConnection({a:'hidden',b:'hidden'})))).overlay,false);
  assert.equal((await activeBrowserPage(uiBrowser(uiConnection({a:'hidden',b:'visible'})))).overlay,true);
  assert.equal((await activeBrowserPage(uiBrowser(uiConnection({a:'hidden',b:'prerender'})))).overlay,true);
  assert.equal((await activeBrowserPage(uiBrowser(uiConnection({a:'hidden'},{fail:true})))).overlay,true);
  const broken=uiConnection({});const send=broken.send;broken.send=async(method,...rest)=>{if(method==='Target.getTargets')throw new Error('x');return send(method,...rest)};
  assert.equal((await activeBrowserPage(uiBrowser(broken))).overlay,true);
});
