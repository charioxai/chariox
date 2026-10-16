// MP-08/MP-10/MP-11: native read-only state never waits on a known loading renderer or creates a pixel/input proof.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
for(const pending of [false,true])test('MP-08/MP-10/MP-11 known native metadata stays responsive with pending='+pending,async()=>{
 const connection={isOpen:()=>true,send:async()=>assert.fail('metadata does not ask the renderer')},browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 browser.connection=connection;browser.ensureTargetSession=async()=> 's';browser.sessionsByTarget.set('t','s');browser.viewportByTarget.set('t','{}');browser.documentIdsByTarget.set('t','doc');browser.readFocus=()=>assert.fail('state metadata never reads page focus');
 if(!pending)browser.pageFramesByTarget.set('t',{connection,sessionId:'s',frame:{id:'frame',loaderId:'doc'}});
 const result=await browser.inspectPage(connection,{targetId:'t',url:'https://html.spec.whatwg.org/',title:'HTML'}, {},{visibleTargetId:'other'});
 assert.equal(result.document_id,'doc');assert.equal(result.focused,false);assert.equal(result.url,'https://html.spec.whatwg.org/');assert.equal(browser.pageFramesByTarget.has('t'),!pending,'pending metadata never admits a document owner');
});

import {nativeStateObservationOptions} from './kernel-browser-display-document.mjs';
for(const mode of ['ordinary','cold','metrics-changed'])test('MP-08/MP-11 '+mode+' state retains renderer discovery',async()=>{
 const calls=[],connection={isOpen:()=>true,send:async method=>{calls.push(method);if(method==='Page.getFrameTree')throw new Error('renderer discovery reached');return {};}};
 const browser=new BrowserCdpClient({connectionFactory:async()=>connection});browser.connection=connection;browser.ensureTargetSession=async()=> 's';browser.sessionsByTarget.set('t','s');browser.viewportByTarget.set('t',mode==='metrics-changed'?'old':'{}');
 if(mode!=='cold')browser.documentIdsByTarget.set('t','doc');
 await assert.rejects(browser.inspectPage(connection,{targetId:'t'},{},mode==='ordinary'?undefined:{visibleTargetId:'t'}),/renderer discovery reached/);
 assert(calls.includes('Page.getFrameTree'));
 if(mode==='metrics-changed')assert.equal(calls[0],'Emulation.setDeviceMetricsOverride');
});
test('MP-08/MP-11 only native read-only state observes the active tab',async()=>{
 let reads=0;const host={chromium:{desktop:{binding:()=>({})}},tabGroups:{active:async()=>{reads++;return 'native-active';}}};
 assert.equal(await nativeStateObservationOptions(host,false),undefined);assert.equal(reads,0,'mutations never select metadata-only inspection');
 assert.deepEqual(await nativeStateObservationOptions(host,true),{visibleTargetId:'native-active'});assert.equal(reads,1);
 assert.equal(await nativeStateObservationOptions({},true),undefined);
 host.chromium.desktop.binding=()=>null;assert.equal(await nativeStateObservationOptions(host,true),undefined);assert.equal(reads,1);
 host.chromium.desktop.binding=()=>({});host.tabGroups.active=async()=>{throw new Error('native binding retired');};assert.equal(await nativeStateObservationOptions(host,true),undefined);
});
