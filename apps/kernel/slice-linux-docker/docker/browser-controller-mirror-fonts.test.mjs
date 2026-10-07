import test from 'node:test';
import assert from 'node:assert/strict';
import {LoadedMirrorFonts} from './browser-controller-mirror-fonts.mjs';
const response=(session='s',loader='d',url='https://example.test/font',request='r')=>({sessionId:session,method:'Network.responseReceived',params:{type:'Font',requestId:request,loaderId:loader,response:{url,status:200}}});
const finish=(request='r')=>({sessionId:'s',method:'Network.loadingFinished',params:{requestId:request}});
test('only finished Font bodies from the exact session/document can be read',async()=>{
 const fonts=new LoadedMirrorFonts(),calls=[];const connection={async send(...args){calls.push(args);return {base64Encoded:true,body:'AA=='}}};
 fonts.observe({...response(),params:{...response().params,type:'Image'}});assert.equal(await fonts.read(connection,'s','https://example.test/font','d'),null);
 fonts.observe(response());assert.equal(await fonts.read(connection,'s','https://example.test/font','d'),null);fonts.observe(finish());
 assert.equal(await fonts.read(connection,'s','https://example.test/font','other'),null);assert.equal(await fonts.read(connection,'foreign','https://example.test/font','d'),null);
 assert.deepEqual(await fonts.read(connection,'s','https://example.test/font','d'),{base64Encoded:true,content:'AA=='});assert.deepEqual(calls,[['Network.getResponseBody',{requestId:'r'},'s']]);
});
test('navigation or response replacement during a body read refuses the old bytes',async()=>{
 for(const changed of ['navigation','replacement','detach']){
  const fonts=new LoadedMirrorFonts();fonts.observe(response());fonts.observe(finish());const ready=Promise.withResolvers(),connection={send:()=>ready.promise};const read=fonts.read(connection,'s','https://example.test/font','d');
  if(changed==='navigation')fonts.observe({sessionId:'s',method:'Page.frameNavigated',params:{frame:{loaderId:'new'}}});else if(changed==='replacement')fonts.observe(response('s','d','https://example.test/font','new'));else fonts.removeSession('s');
  ready.resolve({body:'AA==',base64Encoded:true});await assert.rejects(read,/changed/);
 }
});
test('failed loads and a byte-budget overflow evict only cached lookup metadata',async()=>{
 const fonts=new LoadedMirrorFonts();fonts.observe(response());fonts.observe({sessionId:'s',method:'Network.loadingFailed',params:{requestId:'r'}});assert.equal(fonts.sessions.get('s').size,0);
 for(let i=0;i<200;i++)fonts.observe(response('s','d','https://example.test/'+String(i)+'x'.repeat(7900),'r'+i));let bytes=0;for(const [url,entry]of fonts.sessions.get('s'))bytes+=url.length*2+entry.requestId.length*2+entry.loaderId.length*2+64;assert(bytes<=1024*1024);assert(!fonts.sessions.get('s').has('https://example.test/0'+'x'.repeat(7900)));fonts.clear();assert.equal(fonts.sessions.size,0);
});
