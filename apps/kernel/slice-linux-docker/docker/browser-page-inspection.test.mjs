// MP-08/MP-10/MP-11: a delayed observation must never replace the event-owned newer document.
import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserCdpClient} from './browser-controller-cdp.mjs';
test('MP-11 late page inspection cannot revive a retired document owner',async()=>{
 const ready=Promise.withResolvers(),connection={isOpen:()=>true,send:async method=>{assert.equal(method,'Page.getFrameTree');return {frameTree:{frame:{id:'frame',loaderId:'old'}}};}};
 const browser=new BrowserCdpClient({connectionFactory:async()=>connection});browser.connection=connection;browser.ensureTargetSession=async()=> 's';browser.sessionsByTarget.set('t','s');browser.viewportByTarget.set('t','{}');browser.readFocus=()=>ready.promise;
 const old={connection,sessionId:'s',frame:{id:'frame',loaderId:'old'}};browser.documentIdsByTarget.set('t','old');browser.pageFramesByTarget.set('t',old);
 const inspected=browser.inspectPage(connection,{targetId:'t',url:'https://en.wikipedia.org/wiki/HTML',title:'HTML'},{});
 await new Promise(setImmediate);
 const current={connection,sessionId:'s',frame:{id:'frame',loaderId:'new'}};browser.documentIdsByTarget.set('t','new');browser.pageFramesByTarget.set('t',current);ready.resolve(false);
 const result=await Promise.allSettled([inspected]);
 assert.equal(browser.currentPageFrame('t'),current,'the event-owned new document survives the late observation');
 assert.equal(result[0].status,'rejected');assert.equal(result[0].reason.code,'stale_document_reference');
});
for(const change of ['same-loader','session-retired','connection-replaced'])test('MP-11 delayed inspection keeps its owner retired after '+change,async()=>{
 const ready=Promise.withResolvers(),connection={isOpen:()=>true,send:async()=>({frameTree:{frame:{id:'frame',loaderId:'doc'}}})},browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 browser.connection=connection;browser.ensureTargetSession=async()=> 's';browser.sessionsByTarget.set('t','s');browser.viewportByTarget.set('t','{}');browser.readFocus=()=>ready.promise;
 const frame={connection,sessionId:'s',frame:{id:'frame',loaderId:'doc'}};browser.pageFramesByTarget.set('t',frame);browser.documentIdsByTarget.set('t','doc');
 const inspected=browser.inspectPage(connection,{targetId:'t'},{});await new Promise(setImmediate);
 let expected;
 if(change==='same-loader'){expected={...frame};browser.pageFramesByTarget.set('t',expected);}
 else{browser.pageFramesByTarget.clear();browser.documentIdsByTarget.clear();if(change==='session-retired')browser.sessionsByTarget.clear();else browser.connection={isOpen:()=>true};}
 ready.resolve(false);await assert.rejects(inspected,e=>e.code==='stale_document_reference');assert.equal(browser.pageFramesByTarget.get('t'),expected);
});
test('MP-08/MP-11 fresh cold inspection repairs an unmatched old cache without trusting it',async()=>{
 const connection={isOpen:()=>true,send:async()=>({frameTree:{frame:{id:'frame',loaderId:'new'}}})},browser=new BrowserCdpClient({connectionFactory:async()=>connection});
 browser.connection=connection;browser.ensureTargetSession=async()=> 's';browser.sessionsByTarget.set('t','s');browser.viewportByTarget.set('t','{}');browser.readFocus=async()=>false;
 browser.pageFramesByTarget.set('t',{connection:{},sessionId:'old-session',frame:{id:'frame',loaderId:'old'}});browser.documentIdsByTarget.set('t','old');
 await browser.inspectPage(connection,{targetId:'t'},{});assert.equal(browser.currentPageFrame('t').frame.loaderId,'new');assert.equal(browser.currentPageFrame('t').connection,connection);
});
