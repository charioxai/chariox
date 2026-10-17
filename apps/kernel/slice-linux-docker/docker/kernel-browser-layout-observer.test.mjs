// MP-08/MP-10/MP-11: the page-side layout observer retires the cached layout verdict on every signal
// that can change the layout viewport, zoom or scale. Page-side triggers run in a real Chrome.
import test from 'node:test';
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,readFile,rm} from 'node:fs/promises';
import {existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {observeLayout,MAX_AGE_MS,observerSource} from './kernel-browser-layout-observer.mjs';

// Fake connection: records installs and lets a test emit CDP events.
const fake=({fail=false}={})=>{
  const listeners=new Set(),sent=[];
  return {sent,
    subscribe:fn=>{listeners.add(fn);return()=>listeners.delete(fn);},
    emit:message=>[...listeners].forEach(fn=>fn(message)),
    listeners,
    async send(method,params,sessionId){
      sent.push({method,params,sessionId});
      if(fail&&method==='Runtime.evaluate')return {exceptionDetails:{text:'boom'}};
      if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f'}}};
      if(method==='Page.createIsolatedWorld')return {executionContextId:7};
      return {};
    }};
};
const bindingOf=c=>c.sent.find(s=>s.method==='Runtime.addBinding').params.name;
const report=(c,name,extra={})=>c.emit({sessionId:'s',method:'Runtime.bindingCalled',params:{name,executionContextId:7,payload:'1',...extra}});
const ready=async(c=fake(),now=()=>0)=>{const o=await observeLayout(c,'s',now);await o.confirm(async()=>true);assert.equal(o.valid(),true);return [o,c];};

test('MP-11 the verdict is unproven until a fresh read matched and is reused only afterwards',async()=>{
  const c=fake(),o=await observeLayout(c,'s',()=>0);
  assert.equal(o.valid(),false,'a new observer has no verdict');
  assert.equal(await o.confirm(async()=>false),false);assert.equal(o.valid(),false,'a mismatching read never becomes the verdict');
  assert.equal(await o.confirm(async()=>true),true);assert.equal(o.valid(),true);
  assert.deepEqual(c.sent.map(s=>s.method),['Page.getFrameTree','Runtime.addBinding','Page.createIsolatedWorld','Runtime.evaluate']);
  assert.equal(c.sent[1].params.executionContextName,c.sent[2].params.worldName,'the binding lives only in the private world');
});
test('MP-11 every page report retires the verdict; an in-flight read cannot confirm across one',async()=>{
  const [o,c]=await ready();
  report(c,bindingOf(c));assert.equal(o.valid(),false);
  assert.equal(await o.confirm(async()=>{report(c,bindingOf(c));return true;}),true);
  assert.equal(o.valid(),false,'a report during the read leaves the read unconfirmed');
  assert.equal(await o.confirm(async()=>true),true);assert.equal(o.valid(),true);
  report(c,'another-binding');report(c,bindingOf(c),{executionContextId:99});assert.equal(o.valid(),true,'foreign bindings and contexts are ignored');
});
test('MP-11 the verdict is never older than MAX_AGE_MS',async()=>{
  let t=1000;const [o]=await ready(fake(),()=>t);
  t+=MAX_AGE_MS-1;assert.equal(o.valid(),true);t+=1;assert.equal(o.valid(),false);
});
for(const [name,message]of [
  ['top-level navigation',{sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'f'}}}],
  ['cleared execution contexts',{sessionId:'s',method:'Runtime.executionContextsCleared',params:{}}],
  ['destroyed observer context',{sessionId:'s',method:'Runtime.executionContextDestroyed',params:{executionContextId:7}}],
  ['target detach',{method:'Target.detachedFromTarget',params:{sessionId:'s'}}],
  ['inspector detach',{sessionId:'s',method:'Inspector.detached',params:{}}],
  ['browser disconnect',{method:'Chariox.browserDisconnected',params:{}}],
])test(`MP-11 ${name} retires the observer for good`,async()=>{
  const [o,c]=await ready();
  c.emit(message);assert.equal(o.valid(),false);
  await o.confirm(async()=>true);assert.equal(o.valid(),false,'a dead observer never confirms again');assert.equal(c.listeners.size,0);
});
test('MP-11 child frames and other sessions do not retire the verdict',async()=>{
  const [o,c]=await ready();
  c.emit({sessionId:'s',method:'Page.frameNavigated',params:{frame:{id:'child',parentId:'f'}}});
  c.emit({sessionId:'other',method:'Page.frameNavigated',params:{frame:{id:'x'}}});
  c.emit({sessionId:'other',method:'Runtime.executionContextsCleared',params:{}});
  c.emit({sessionId:'s',method:'Runtime.executionContextDestroyed',params:{executionContextId:8}});
  assert.equal(o.valid(),true);
});
test('MP-11 a failed install or a connection without events leaves no observer, so every publication reads',async()=>{
  const failing=fake({fail:true});assert.equal(await observeLayout(failing,'s'),null);assert.equal(failing.listeners.size,0);
  assert.equal(await observeLayout({send:async()=>({})},'s'),null);
});
test('MP-11 close removes the listener and the binding',async()=>{
  const [o,c]=await ready();o.close();
  assert.equal(c.listeners.size,0);assert.equal(o.valid(),false);
  assert.ok(c.sent.some(s=>s.method==='Runtime.removeBinding'&&s.params.name===bindingOf(c)));
});

// --- real Chrome: each page-side trigger reaches Node as a report ---
const chrome=process.env.CHARIOX_TEST_CHROMIUM||(existsSync('/usr/bin/google-chrome')?'/usr/bin/google-chrome':null);
const real=chrome?test:test.skip;
async function launch(){
  const dir=await mkdtemp(path.join(process.env.TMPDIR??tmpdir(),'layout-observer-'));
  const child=spawn(chrome,['--headless=new','--no-sandbox','--disable-gpu','--remote-debugging-port=0',`--user-data-dir=${dir}`,'--window-size=800,600','about:blank'],{stdio:'ignore'});
  let port,browserPath;
  for(let i=0;i<100&&!port;i++){
    try{[port,browserPath]=(await readFile(path.join(dir,'DevToolsActivePort'),'utf8')).split('\n');}catch{await new Promise(r=>setTimeout(r,100));}
  }
  assert.ok(port,'chrome did not start');
  const ws=new WebSocket(`ws://127.0.0.1:${port}${browserPath}`),pending=new Map(),listeners=new Set();let id=0;
  await new Promise((resolve,reject)=>{ws.onopen=resolve;ws.onerror=reject;});
  ws.onmessage=event=>{
    const m=JSON.parse(event.data);
    if(m.id&&pending.has(m.id)){const [resolve,reject]=pending.get(m.id);pending.delete(m.id);m.error?reject(Error(m.error.message)):resolve(m.result);}
    else listeners.forEach(fn=>fn(m));
  };
  const connection={
    send:(method,params={},sessionId)=>new Promise((resolve,reject)=>{pending.set(++id,[resolve,reject]);ws.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));}),
    subscribe:fn=>{listeners.add(fn);return()=>listeners.delete(fn);},
  };
  const {targetInfos}=await connection.send('Target.getTargets');
  const target=targetInfos.find(t=>t.type==='page');
  const {sessionId}=await connection.send('Target.attachToTarget',{targetId:target.targetId,flatten:true});
  for(const domain of ['Page','Runtime'])await connection.send(`${domain}.enable`,{},sessionId);
  const page=expression=>connection.send('Runtime.evaluate',{expression,awaitPromise:true},sessionId);
  return {connection,sessionId,page,async close(){ws.close();child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));await rm(dir,{recursive:true,force:true,maxRetries:10,retryDelay:100});}};
}
const settle=ms=>new Promise(r=>setTimeout(r,ms));
real('MP-11 real Chrome: every layout-changing signal reaches the observer, scrolling alone does not',async()=>{
  const browser=await launch();
  try{
    const {connection,sessionId,page}=browser;
    await page(`document.body.style.margin='0';document.body.innerHTML='<div style="height:2000px">tall</div>';`);
    const observer=await observeLayout(connection,sessionId,()=>0);
    assert.ok(observer,'the observer installs in the real page');
    await settle(200); // ResizeObserver reports the initial observation once
    const confirm=async()=>{await observer.confirm(async()=>true);assert.equal(observer.valid(),true);};
    const triggers={
      'DOM mutation':()=>page(`document.body.append(document.createElement('p'))`),
      'attribute change':()=>page(`document.body.setAttribute('data-x','1')`),
      'text change':()=>page(`document.body.firstChild.firstChild.data='changed'`),
      'window resize':()=>connection.send('Emulation.setDeviceMetricsOverride',{width:701,height:500,deviceScaleFactor:1,mobile:false},sessionId),
      'pinch scale (visual viewport)':()=>connection.send('Emulation.setPageScaleFactor',{pageScaleFactor:1.5},sessionId),
      'stylesheet-only layout change (no DOM mutation)':()=>page(`(()=>{const s=new CSSStyleSheet();s.replaceSync('body{width:640px;height:3000px}');document.adoptedStyleSheets=[...document.adoptedStyleSheets,s];})()`),
      'CSS transition':()=>page(`(()=>{const e=document.createElement('i');e.style.cssText='display:block;width:1px;transition:width 30ms';document.body.append(e);})()`),
      'font load':()=>page(`(async()=>{const f=new FontFace('observer-probe','local("Arial"), local("DejaVu Sans"), local("Liberation Sans")');document.fonts.add(f);await f.load();document.fonts.dispatchEvent?.call(document.fonts,new Event('loadingdone'));})()`),
    };
    for(const [name,trigger]of Object.entries(triggers)){
      await settle(150); // let the previous trigger's later reports (ResizeObserver runs per frame) land
      await confirm();await settle(30);assert.equal(observer.valid(),true,`${name}: nothing happened yet`);
      await trigger();
      for(let i=0;i<40&&observer.valid();i++)await settle(25);
      assert.equal(observer.valid(),false,`${name} must retire the verdict`);
    }
    await settle(200);await confirm();
    await page(`scrollTo(0,500)`);await page(`scrollTo(0,900)`);await settle(150);
    assert.equal(observer.valid(),true,'scrolling alone cannot change the layout viewport');
    await connection.send('Page.navigate',{url:'data:text/html,<p>next'},sessionId);
    for(let i=0;i<40&&observer.valid();i++)await settle(25);
    assert.equal(observer.valid(),false,'navigation retires the observer');
    observer.close();
  }finally{await browser.close();}
});
real('MP-11 real Chrome: the observer world is invisible to page scripts',async()=>{
  const browser=await launch();
  try{
    const {connection,sessionId,page}=browser;
    const observer=await observeLayout(connection,sessionId,()=>0);
    const names=(await page(`Object.getOwnPropertyNames(globalThis).filter(n=>/charioxLayout/i.test(n)).join()`)).result.value;
    assert.equal(names,'','the binding is not exposed to the page main world');
    assert.ok(observerSource('x').includes('globalThis["x"]'));
    observer.close();
  }finally{await browser.close();}
});
