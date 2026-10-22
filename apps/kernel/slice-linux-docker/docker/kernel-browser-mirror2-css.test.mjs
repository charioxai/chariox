// MP-08/MP-10/MP-11 M2: CSS agent resource loading must not gate large-DOM paint.
import test from 'node:test';
import assert from 'node:assert/strict';
import { Mirror2 } from './kernel-browser-mirror2.mjs';

test('MP-10 M2: a large document world is ready while CSS.enable waits for resources', async () => {
  let ready,listener;const calls=[];
  const cssReady=new Promise(resolve=>{ready=resolve});
  const connection={subscribe(fn){listener=fn;return()=>{}},async send(method,params){calls.push({method,params});if(method==='Page.getFrameTree')return{frameTree:{frame:{id:'F',loaderId:'D'}}};if(method==='Runtime.evaluate')return{result:{value:true}};if(method==='CSS.enable')return cssReady;return{};}};
  const browser={async resolvePageTarget(){return{connection,sessionId:'S'}},async ensureFocusWorld(){return{contextId:1}}};
  const mirror=new Mirror2({host:{browser}});
  let timer;
  try{const world=await Promise.race([mirror.world({target_id:'T',document_id:'D'},{id:'s'}),new Promise((_,reject)=>{timer=setTimeout(()=>reject(Error('CSS resource loading blocked first paint')),100)})]);
    assert.equal(world.contextId,1);assert(calls.some(c=>c.method==='CSS.enable'));
    listener({sessionId:'S',method:'CSS.styleSheetAdded',params:{header:{styleSheetId:'sheet',frameId:'F',isConstructed:true}}});
    ready({});await new Promise(resolve=>setTimeout(resolve,20));
    assert(calls.some(c=>c.method==='Runtime.evaluate'&&c.params.expression.includes("sheetChanged(\"adopted\")")),'styles changed during startup are reconciled after enable');
  }finally{clearTimeout(timer);ready({});mirror.css.release('s');}
});

for(const cached of [true,false])test(`MP-08/MP-10/MP-11 stylesheet fragments are not resource transport keys (cached=${cached})`,async()=>{
  const href='https://example.test/assets/site.css?bundle=a#client-flags',transport=href.split('#')[0],css='.hit{color:red}',calls=[];
  const mirror=new Mirror2({host:{}});
  mirror.resourceBody=async(_world,url)=>{calls.push(url);return cached&&url===transport?Buffer.from(css):null};
  mirror.evaluate=async(_world,expression)=>{assert(expression.includes(css));assert(expression.includes(href),'the original stylesheet base is retained for CSS URL resolution');return 'normalized CSS'};
  const world={sessionId:'s',connection:{async send(method,params){if(method==='Page.getResourceTree')return{frameTree:{frame:{id:'f'},resources:[{url:transport}]}};assert.equal(method,'Page.getResourceContent');assert.equal(params.url,transport);return{content:css}}}};
  assert.equal(await mirror.crossOriginSheet(world,href),'normalized CSS');
  assert.deepEqual(calls,[transport]);
});
