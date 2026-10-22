// MP-08/MP-11: authority-sensitive native dispatch, without page values.
import test from "node:test";
import assert from "node:assert/strict";
import { inputHostTab } from "./kernel-browser-input.mjs";

function fixture(protectedTarget = false) {
  const sent = [];
  const connection = { send: async (method, params) => {
    sent.push({ method, params });
    if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "f", loaderId: "doc" } } };
    if (method === "Page.createIsolatedWorld") return { executionContextId: 7 };
    if (method === "Runtime.evaluate") return { result: { value: protectedTarget } };
    return {};
  } };
  return { sent, browser: { resolvePageTarget: async () => ({ connection, sessionId: "s" }), inputCapture: { run: async (_c, _s, fn) => fn() } } };
}
const tab = { target_id: "t", document_id: "doc" };
test("MP-11: retained typing, keys and clicks dispatch identically to focused input", async () => {
  for (const input of [{kind:'click',x:1,y:2},{kind:'text',text:'fixture'},
    ...['Tab','Shift+Tab','Enter','Space','Delete','Backspace','ArrowLeft','Home','a','Escape'].map(key=>({kind:'key',key}))]) {
    const focused=fixture(), retained=fixture();
    await inputHostTab(focused.browser,tab,input);
    await inputHostTab(retained.browser,tab,input,{retained:true});
    assert.deepEqual(retained.sent,focused.sent);
    assert(retained.sent.some(({method})=>method.startsWith('Input.')));
    assert(!retained.sent.some(({method})=>method.startsWith('DOMDebugger.')));
  }
});
test("MP-08: retained wheel and focused clicks proceed",async()=>{
  const {browser,sent}=fixture(false);
  await inputHostTab(browser,tab,{kind:'scroll',x:1,y:2,delta_x:0,delta_y:100},{retained:true});
  await inputHostTab(browser,tab,{kind:'click',x:1,y:2});
  assert.deepEqual(sent.filter(call=>call.method.startsWith('Input.')).map(call=>call.params.type),['mouseWheel','mousePressed','mouseReleased']);
});
test("MP-08/MP-11: focused paired Tab releases before reporting revocation",async()=>{
  const {browser,sent}=fixture(false);
  const cancellation=new AbortController();
  const {connection}=await browser.resolvePageTarget();
  const send=connection.send;
  connection.send=async(method,params)=>{
    const result=await send(method,params);
    if(method==='Input.dispatchKeyEvent' && params.type==='keyDown') cancellation.abort();
    return result;
  };
  await assert.rejects(inputHostTab(browser,tab,{kind:'key',key:'Tab'},{signal:cancellation.signal}),{code:'browser_action_cancelled'});
  assert.equal(sent.filter(call=>call.method==='Input.dispatchKeyEvent').length,2);
});

// A-L23: typing is the person's own (or a benign agent's); secrets on screen are masked at egress, not by refusing keys.
test("A-L23: text, keys and chords reach password, one-time-code and card fields without a focus read", async () => {
  for (const retained of [false, true]) for (const input of [
    {kind:'text',text:'fixture'}, ...['a','é','😀',' ', 'Enter','Space'].map(key=>({kind:'key',key})), {kind:'key',key:'v',modifiers:2},
  ]) {
    const {browser,sent}=fixture(true);
    await inputHostTab(browser,tab,input,{retained});
    assert(sent.some(({method})=>method.startsWith('Input.')),JSON.stringify(input));
    assert(!sent.some(({method})=>method==='Runtime.evaluate'),'no page read before a key');
  }
});
test("MP-08/MP-11: protected targets keep non-text navigation", async () => {
  for (const key of ['Tab','Shift+Tab','ArrowLeft','ArrowRight','Home','End','Escape']) {
    const {browser,sent}=fixture(true);
    await inputHostTab(browser,tab,{kind:'key',key});
    assert.deepEqual(sent.filter(({method})=>method.startsWith('Input.')).map(({params})=>params.type),['keyDown','keyUp']);
  }
});

test('MP-11: an asynchronous live mirror guard fences physical dispatch',async()=>{
  let release,entered,dispatches=0,guards=0,captured=false;
  const held=new Promise(resolve=>release=resolve),started=new Promise(resolve=>entered=resolve);
  const connection={async send(method){if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'frame',loaderId:'d'}}};assert.equal(method,'Input.dispatchMouseEvent');return {};}};
  const browser={async resolvePageTarget(){return {connection,sessionId:'s'}},inputCapture:{async run(_connection,_session,operation){captured=true;return operation()}}};
  const pending=inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'mirror'},{onDispatch(){dispatches++},resolveMirror:async()=>({input:{kind:'click',x:10,y:10},guard(){assert(captured);guards++;entered();return held}})});
  try{await started;await new Promise(resolve=>setImmediate(resolve));assert.equal(dispatches,0,'MP-11: live guard must settle before either physical event');}finally{release();await pending;}
  assert.equal(dispatches,1);assert.equal(guards,1,'MP-08/MP-11: the final admission guard precedes the ordered press/release pair');
});

test('A-L23: IME composition reaches a protected field after the final mirror guard',async()=>{
 const {browser,sent}=fixture(true);let guarded=false;
 await inputHostTab(browser,tab,{kind:'mirror'},{resolveMirror:async()=>({
  guard:async()=>{guarded=true;},
  perform:send=>send('Input.imeSetComposition',{text:'文',selectionStart:1,selectionEnd:1}),
 })});
 assert(guarded);assert.equal(sent.filter(x=>x.method==='Input.imeSetComposition').length,1);
});

test('MP-08/MP-11: native navigation and editing keys carry their Chromium virtual key codes',async()=>{
 const codes={Tab:9,Enter:13,Escape:27,Backspace:8,Delete:46,ArrowLeft:37,ArrowRight:39,ArrowUp:38,ArrowDown:40,Home:36,End:35};
 for(const [key,code]of Object.entries(codes)){
   const events=[];const connection={async send(method,params){if(method==='Page.getFrameTree')return {frameTree:{frame:{loaderId:'d'}}};if(method==='Input.dispatchKeyEvent'){events.push(params);return {}};if(method==='Page.createIsolatedWorld')return {executionContextId:7};if(method==='Runtime.evaluate')return {result:{value:false}};throw Error(`unexpected ${method}`)}};
   const browser={async resolvePageTarget(){return {connection,sessionId:'s'}},inputCapture:{run(_c,_s,fn){return fn()}}};
   await inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'key',key});
   assert.deepEqual(events.map(e=>e.windowsVirtualKeyCode),[code,code]);assert.deepEqual(events.map(e=>e.type),['keyDown','keyUp']);
 }
});
// MP-08/MP-10: a deferred wheel ack lets the next ordered input dispatch while
// the renderer acknowledges the previous one; the document fence still runs first.
test('MP-08/MP-10 asynchronous scroll returns after an ordered, fenced dispatch without the renderer ack',async()=>{
  const {browser,sent}=fixture();const {connection}=await browser.resolvePageTarget();const send=connection.send;
  let release;const acked=new Promise(r=>release=r);
  connection.send=async(method,params)=>{if(method==='Input.dispatchMouseEvent'){sent.push({method,params});await acked;return {}}return send(method,params)};
  let dispatched=0;
  const result=await inputHostTab(browser,tab,{kind:'scroll',x:10,y:10,delta_x:0,delta_y:120},{asyncScroll:true,onDispatch:()=>{dispatched++;}});
  assert.ok(result.ack instanceof Promise,'lane work returns before the renderer ack');
  assert.equal(dispatched,1);
  const methods=sent.map(x=>x.method);assert.ok(methods.indexOf('Page.getFrameTree')<methods.indexOf('Input.dispatchMouseEvent'),'document fence precedes dispatch');
  release();await result.ack;
  assert.equal(await inputHostTab(browser,tab,{kind:'scroll',x:10,y:10,delta_x:0,delta_y:120}),undefined,'synchronous callers still await the ack');
});
// r55: the page session's ordered navigation events prove the document, so a person's native wheel is not held 5 s by a
// busy renderer's Page.getFrameTree. Without a proof (a navigation in flight) the renderer is still asked.
test('MP-08/MP-10/MP-11 a native wheel is fenced by the ordered navigation events when they prove the document',async()=>{
  const {browser,sent}=fixture();const {connection}=await browser.resolvePageTarget();const wheels=[];
  browser.currentPageFrame=()=>({connection,sessionId:'s',frame:{id:'f',loaderId:'doc'}});
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:-240},{nativeWheel:(...a)=>{wheels.push(a);return true}});
  assert.deepEqual(wheels,[[10,20,0,-2]]);assert.equal(sent.some(x=>x.method==='Page.getFrameTree'),false,'no renderer round trip');
  browser.currentPageFrame=()=>({connection,sessionId:'s',frame:{id:'f',loaderId:'moved'}});
  await assert.rejects(inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:-240},{nativeWheel:()=>true}),error=>error.code==='stale_document_reference');
  assert.equal(sent.some(x=>x.method==='Page.getFrameTree'),false,'a moved document is refused from the events too');
  browser.currentPageFrame=()=>undefined;wheels.length=0;
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:-240},{nativeWheel:(...a)=>{wheels.push(a);return true}});
  assert.equal(sent.some(x=>x.method==='Page.getFrameTree'),true,'no proof: the renderer is asked');
});
// r55: a wheel needs no page focus: it must not wait for the renderer to re-enable focus emulation (5 s on a busy page).
test('MP-08/MP-10/MP-11 a wheel does not resume focus emulation before it is sent, a key does',async()=>{
  const {browser}=fixture();const options=[];
  browser.inputCapture.run=async(_c,_s,fn,opts)=>{options.push(opts);return fn()};
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{});
  await inputHostTab(browser,tab,{kind:'key',key:'Tab'},{});
  assert.equal(options[0]?.resumeFocus,false);assert.notEqual(options[1]?.resumeFocus,false);
});
// MP-08/MP-10: wheel notches use the owned display after the same fences;
// unavailable native input keeps precise CDP deltas or fails closed.
test('MP-08/MP-10 notch wheel input routes to the owned display after the document fence',async()=>{
  const {browser,sent}=fixture();const wheels=[];let dispatched=0;
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:-240},{nativeWheel:(...a)=>{wheels.push(a);return true},onDispatch:()=>{dispatched++;}});
  assert.deepEqual(wheels,[[10,20,0,-2]]);assert.equal(dispatched,1);
  assert.ok(sent.some(x=>x.method==='Page.getFrameTree'),'document fence ran');assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,0);
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:37},{nativeWheel:()=>false});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,1);
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{nativeWheel:()=>false});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,2,'MP-11: refused native input falls back to CDP');
});

// MP-08/MP-10: only exact mouse-wheel steps use native notches.
test('MP-08/MP-10 viewer mouse-wheel steps map to native notches',async()=>{
 const {notches}=await import('./kernel-browser-input.mjs');
 assert.deepEqual([120,-240,100,-100,200,53,-80,0].map(notches),[1,-2,1,-1,2,null,null,0]);
 assert.deepEqual([1,-12,37,49].map(notches),[null,null,null,null]);
});

test('MP-08/MP-10: precise wheel motion preserves its distance and direction',async()=>{
 const {browser,sent}=fixture(),wheels=[];let dispatched=0;
 const deltas=[1,16,37,49,53,80,-1,-16,-37,-49,-53,-80];
 for(const delta_y of deltas)await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y},{nativeWheel:(...args)=>{wheels.push(args);return true;},onDispatch:()=>{dispatched++;}});
 assert.deepEqual(wheels,[],'a trackpad pixel must not become a whole native wheel notch');
 assert.deepEqual(sent.filter(call=>call.method==='Input.dispatchMouseEvent').map(call=>call.params.deltaY),deltas);
 assert.equal(dispatched,deltas.length);
 assert.ok(sent.some(call=>call.method==='Page.getFrameTree'),'the document fence is retained');
});
// MP-08/MP-10: viewer clicks on the owned display use XTest after the fence.
test('MP-08/MP-10 viewer click routes to the owned display after the document fence; refusal falls back to CDP',async()=>{
  const {browser,sent}=fixture();const clicks=[];let dispatched=0;
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:(...a)=>{clicks.push(a);return true},onDispatch:()=>{dispatched++;}});
  assert.deepEqual(clicks,[[10,20]]);assert.equal(dispatched,1);
  assert.ok(sent.some(x=>x.method==='Page.getFrameTree'),'document fence ran');assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,0);
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:()=>false});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,2,'MP-11: refused native click falls back to CDP press/release');
  // MP-11 (review #893 P2): the worker's asynchronous refusal also falls back.
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:async()=>false,onDispatch:()=>{dispatched++;}});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,4,'MP-11: a covered owned window refuses; CDP reaches the renderer');
  // MP-11 (review #893 @8067044d1 P2): a native click/wheel whose reply was
  // lost may have been dispatched: it is never replayed via CDP.
  const uncertain=async()=>{throw Object.assign(Error('MP-11: native input outcome uncertain'),{code:'native_input_uncertain'})};
  const before=dispatched;
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:uncertain,onDispatch:()=>{dispatched++;}});
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{nativeWheel:uncertain,onDispatch:()=>{dispatched++;}});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,4,'MP-11: no CDP replay of an uncertain native action');
  assert.equal(dispatched,before+2,'the uncertain action counts as dispatched once');
});
// r67: a busy renderer (a snapshot in flight) answers document.hasFocus() late. Once a read has proven the page holds
// focus for this document, the next keys do not wait for it while Chrome's own UI holds no claim.
test('MP-08/MP-10 r67 a known page focus does not wait for a busy renderer',async()=>{
  const delay=ms=>new Promise(resolve=>setTimeout(resolve,ms));
  const state={documentId:'doc'};
  const connection={send:async(method)=>{
    if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:state.documentId}}};
    if(method==='Page.createIsolatedWorld')return {executionContextId:7};
    if(method==='Runtime.evaluate'){await delay(400);return {result:{value:true}};}
    return {};}};
  const browser={resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{run:async(_c,_s,fn)=>fn()}};
  const press=async(chromeFocused=()=>false)=>{
    const keys=[],at=performance.now();
    await inputHostTab(browser,{target_id:'t',document_id:state.documentId},{kind:'key',key:'a'},{nativeKey:(...a)=>{keys.push(a);return true},chromeFocused});
    assert.equal(keys.length,1);
    return performance.now()-at;
  };
  assert.ok(await press()>=380,'without a proof the key waits for the read');
  const proven=await press();
  assert.ok(proven<250,`a proven focus does not wait for a busy renderer (${proven|0} ms)`);
  assert.ok(await press(()=>true)>=380,'a Chrome UI focus claim voids the proof');
  state.documentId='doc2';
  assert.ok(await press()>=380,'the proof is for one document');
});
// MP-08/MP-10: viewer keys on the owned display use XTest after the document,
// and focus fences; anything else stays on CDP.
test('MP-08/MP-10 viewer keys route to the owned display only for a focused page',async()=>{
  const run=async({focused=true,native=()=>true,key='a',raw=false})=>{
    const sent=[],keys=[];let dispatched=0;
    const connection={send:async(method,params)=>{sent.push({method,params});
      if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:'doc'}}};
      if(method==='Page.createIsolatedWorld')return {executionContextId:7};
      if(method==='Runtime.evaluate')return {result:{value:focused}};
      return {};}};
    const browser={resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{run:async(_c,_s,fn)=>fn()}};
    const result=await inputHostTab(browser,tab,{kind:'key',key},{nativeKey:(...a)=>{keys.push(a);return native(...a)},onDispatch:()=>{dispatched++;}}).then(()=>'ok',e=>e.code??e.message);
    return {result,keys,dispatched,cdp:sent.filter(x=>x.method==='Input.dispatchKeyEvent').length,...(raw?{sent}:{})};
  };
  assert.deepEqual(await run({}),{result:'ok',keys:[[97,false]],dispatched:1,cdp:0});
  assert.deepEqual(await run({key:'Shift+Tab'}),{result:'ok',keys:[[0xff09,true]],dispatched:1,cdp:0});
  assert.deepEqual(await run({key:'Shift+ArrowLeft'}),{result:'ok',keys:[[0xff51,true]],dispatched:1,cdp:0},'MP-08: review #941-2 Shift extends the selection');
  for (const key of ['Shift+Enter','Shift+Backspace','Shift+a','Ctrl+ArrowLeft','Shift+Shift+Home','Ctrl+a','Ctrl+Shift+Backspace','Ctrl+Ctrl+Delete']) assert.equal((await run({key})).result,'MD-2: unsupported key',key);
  for (const key of ['Ctrl+Backspace','Ctrl+Delete']) {
    const {result,keys,cdp,sent}=await run({key,raw:true});
    assert.deepEqual([result,keys,cdp],['ok',[],2],`MP-08: review #320-2 ${key} deletes a word through CDP (XTest presses no Ctrl)`);
    assert.deepEqual(sent.filter(x=>x.method==='Input.dispatchKeyEvent').map(x=>[x.params.type,x.params.key,x.params.modifiers,x.params.windowsVirtualKeyCode]),[['keyDown',key.slice(5),2,key==='Ctrl+Delete'?46:8],['keyUp',key.slice(5),2,key==='Ctrl+Delete'?46:8]]);
  }
  assert.deepEqual(await run({key:'Enter'}),{result:'ok',keys:[[0xff0d,false]],dispatched:1,cdp:0});
  assert.deepEqual(await run({focused:false}),{result:'ok',keys:[],dispatched:1,cdp:2},'unfocused page (browser UI focus) stays on CDP');
  assert.deepEqual(await run({key:'é'}),{result:'ok',keys:[],dispatched:1,cdp:2},'non-ASCII text stays on CDP');
  assert.deepEqual(await run({native:()=>false}),{result:'ok',keys:[[97,false]],dispatched:1,cdp:2},'MP-11: refused native key falls back to CDP');
});
test("MP-08/MP-10: CDP pointer input follows the emulated view scale (DPR1 page on a scale-2 host)",async()=>{
  // Hosted mirror h3: a DPR1 page on a scale-2 window is emulated at view
  // scale 1/2; unscaled CDP coordinates landed at twice the CSS position.
  const {browser,sent}=fixture(false);
  await inputHostTab(browser,tab,{kind:'click',x:1051,y:541},{viewScale:0.5});
  await inputHostTab(browser,tab,{kind:'scroll',x:100,y:200,delta_x:0,delta_y:120},{viewScale:0.5});
  assert.deepEqual(sent.filter(c=>c.method==='Input.dispatchMouseEvent').map(c=>[c.params.type,c.params.x,c.params.y]),[['mousePressed',525.5,270.5],['mouseReleased',525.5,270.5],['mouseWheel',50,100]]);
  const plain=fixture(false);await inputHostTab(plain.browser,tab,{kind:'click',x:1051,y:541});
  assert.deepEqual(plain.sent.filter(c=>c.method==='Input.dispatchMouseEvent').map(c=>[c.params.x,c.params.y]),[[1051,541],[1051,541]]);
});

// MP-08/MP-10: a press acknowledgement is renderer/frame paced. Its matching
// release must already be ordered on the same CDP connection, not wait a frame.
test('MP-08/MP-10: click release is queued before the press acknowledgement', async () => {
  const { browser, sent } = fixture();
  const { connection } = await browser.resolvePageTarget();
  const send = connection.send;
  const pressed = Promise.withResolvers(), ack = Promise.withResolvers();
  connection.send = async (method, params) => {
    const reply = await send(method, params);
    if (method === 'Input.dispatchMouseEvent' && params.type === 'mousePressed') {
      pressed.resolve(); await ack.promise;
    }
    return reply;
  };
  const pending = inputHostTab(browser, tab, { kind: 'click', x: 10, y: 20 });
  try {
    await pressed.promise; await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(sent.filter(x => x.method === 'Input.dispatchMouseEvent').map(x => x.params.type), ['mousePressed', 'mouseReleased']);
  } finally { ack.resolve(); await pending; }
});

for (const fault of ['cancel', 'navigate', 'press-error'])
test(`MP-08/MP-11: an admitted click pairs its release after ${fault} without replay`, async () => {
  const { browser, sent } = fixture();
  const { connection } = await browser.resolvePageTarget();
  const send = connection.send, cancellation = new AbortController();
  let navigated = false, releaseSettled = false;
  connection.send = async (method, params) => {
    if (method === 'Page.getFrameTree' && navigated) return { frameTree: { frame: { loaderId: 'new' } } };
    const reply = await send(method, params);
    if (method === 'Input.dispatchMouseEvent') {
      if (params.type === 'mousePressed') {
        if (fault === 'cancel') cancellation.abort();
        if (fault === 'navigate') navigated = true;
        if (fault === 'press-error') throw Error('press outcome uncertain');
      } else { await new Promise(resolve => setImmediate(resolve)); releaseSettled = true; }
    }
    return reply;
  };
  const pending = inputHostTab(browser, tab, { kind: 'click', x: 10, y: 20 }, { signal: cancellation.signal });
  if (fault === 'cancel') await assert.rejects(pending, { code: 'browser_action_cancelled' });
  else if (fault === 'press-error') await assert.rejects(pending, /press outcome uncertain/);
  else await pending;
  assert.deepEqual(sent.filter(x => x.method === 'Input.dispatchMouseEvent').map(x => x.params.type), ['mousePressed', 'mouseReleased']);
  assert(releaseSettled, 'the input lane holds ownership until the paired release settles');
});

test('MP-08/MP-10/MP-11: a fenced mirror click leaves its document fence to the live guard, the last call before the press; an unfenced one keeps the input fence', async () => {
  for (const fenced of [true, false]) {
    const { browser, sent } = fixture();
    let captured = false, guardAt = -1;
    browser.inputCapture.run = async (_c, _s, operation) => { captured = true; return operation(); };
    await inputHostTab(browser, tab, { kind: 'mirror' }, { resolveMirror: async () => ({ input: { kind: 'click', x: 10, y: 20 }, fenced, guard: async () => { assert(captured); guardAt = sent.length; } }) });
    const first = sent.findIndex(call => call.method === 'Input.dispatchMouseEvent');
    assert.equal(guardAt, first, 'nothing separates the guard from the press');
    assert.equal(sent.slice(0, first).filter(call => call.method === 'Page.getFrameTree').length, fenced ? 0 : 2);
  }
});

test('MP-08/MP-11: a mirror click without a guard still fences navigation during capture before press', async () => {
  const { browser, sent } = fixture();
  const { connection } = await browser.resolvePageTarget();
  const send = connection.send;
  let captured = false;
  browser.inputCapture.run = async (_c, _s, operation) => { captured = true; return operation(); };
  connection.send = (method, params) => method === 'Page.getFrameTree' && captured ? Promise.resolve({ frameTree: { frame: { loaderId: 'new' } } }) : send(method, params);
  await assert.rejects(inputHostTab(browser, tab, { kind: 'mirror' }, { resolveMirror: async () => ({ input: { kind: 'click', x: 10, y: 20 } }) }), { code: 'stale_document_reference' });
  assert.equal(sent.filter(call => call.method === 'Input.dispatchMouseEvent').length, 0);
});

test('MP-08/MP-10 protocol498 Ctrl+A is a chord with paired releases and no inserted a',async()=>{
 const {browser,sent}=fixture(false);
 await inputHostTab(browser,tab,{kind:'key',key:'a',modifiers:2},{nativeKey:()=>{throw Error('modifiers require fenced CDP')}});
 const events=sent.filter(x=>x.method==='Input.dispatchKeyEvent').map(x=>x.params);
 assert.equal(events.length,2);assert.equal(events[0].modifiers,2);assert.equal(events[1].modifiers,2);
 assert.equal(events[0].windowsVirtualKeyCode,65);assert.equal(events[0].code,'KeyA');assert.equal(events[0].text,undefined);
});
test('A-L23 protocol498 modifier chords reach protected fields',async()=>{
 for(const key of ['a','c','v','x','ArrowLeft'])for(const modifiers of [2,4,10]){
  const {browser,sent}=fixture(true);
  await inputHostTab(browser,tab,{kind:'key',key,modifiers});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchKeyEvent').length,2);
 }
});
test('MP-11 protocol498 malformed modifiers are refused before input',async()=>{
 for(const modifiers of [-1,16,NaN,null,1.5,'2']){
  const {browser,sent}=fixture(false);
  await assert.rejects(inputHostTab(browser,tab,{kind:'key',key:'a',modifiers}),/modifiers/);
  assert.equal(sent.filter(x=>x.method.startsWith('Input.')).length,0);
 }
});
// Review #1016 @f0ccdb5c2 P2: undo/redo in a mirrored iframe editor keeps the mirror's observed-frame admission.
test('MP-08/MP-11 mirrored undo/redo chords keep the mirror\'s observed-frame admission',async()=>{
  for(const modifiers of [2,10]){
    const {browser,sent}=fixture();
    await inputHostTab(browser,tab,{kind:'mirror'},{resolveMirror:async()=>({input:{kind:'key',key:'z',modifiers},guard:async()=>{}})});
    assert.deepEqual(sent.filter(x=>x.method==='Input.dispatchKeyEvent').map(x=>[x.params.type,x.params.code,x.params.modifiers]),[['keyDown','KeyZ',modifiers],['keyUp','KeyZ',modifiers]]);
  }
});

for(const fault of ['navigation','cancel','press-error'])test(`MP-08/MP-10/MP-11 a key pairs its release after ${fault}`,async()=>{
 const {browser,sent}=fixture(false);const {connection}=await browser.resolvePageTarget();const original=connection.send;
 const cancellation=new AbortController();let navigated=false,releaseSettled=false;
 connection.send=async(method,params)=>{
  if(method==='Page.getFrameTree'&&navigated)return {frameTree:{frame:{id:'f',loaderId:'next'}}};
  const result=await original(method,params);
  if(method==='Input.dispatchKeyEvent'){
   if(params.type==='keyDown'){
    navigated=fault==='navigation';if(fault==='cancel')cancellation.abort();
    if(fault==='press-error')throw Error('key press reply failed');
   }else{await new Promise(resolve=>setImmediate(resolve));releaseSettled=true;}
  }return result;
 };
 const task=inputHostTab(browser,tab,{kind:'key',key:'Enter'},{signal:cancellation.signal});
 if(fault==='cancel')await assert.rejects(task,{code:'browser_action_cancelled'});
 else if(fault==='press-error')await assert.rejects(task,/key press reply failed/);
 else await task;
 assert.deepEqual(sent.filter(x=>x.method==='Input.dispatchKeyEvent').map(x=>x.params.type),['keyDown','keyUp']);
 assert(releaseSettled,'release must settle before another actor acquires input');
});

// Coordinator 15:08Z (mdmain r11 Notes on the video fallback): protocol 530 page drag.
test("MP-08/MP-11 (530): a page drag presses, moves and releases in the view scale; the release is queued with the press", async () => {
  const { browser, sent } = fixture(false);
  const { connection } = await browser.resolvePageTarget();
  const send = connection.send, pressed = Promise.withResolvers(), ack = Promise.withResolvers();
  connection.send = async (method, params) => { const reply = await send(method, params); if (params?.type === "mousePressed") { pressed.resolve(); await ack.promise; } return reply; };
  const pending = inputHostTab(browser, tab, { kind: "drag", x: 100, y: 40, to_x: 300, to_y: 60 }, { viewScale: 0.5 });
  try {
    await pressed.promise; await new Promise(resolve => setImmediate(resolve));
    const mouse = sent.filter(x => x.method === "Input.dispatchMouseEvent").map(x => x.params);
    assert.deepEqual(mouse.map(p => p.type), ["mousePressed", ...Array(8).fill("mouseMoved"), "mouseReleased"], "moves and release ordered behind the press");
    assert.deepEqual([mouse[0].x, mouse[0].y, mouse.at(-1).x, mouse.at(-1).y], [50, 20, 150, 30]);
    assert(mouse.slice(1, -1).every(p => p.buttons === 1), "the primary button stays down while moving");
  } finally { ack.resolve(); await pending; }
  await assert.rejects(inputHostTab(browser, tab, { kind: "drag", x: 1, y: 1, to_x: 99999, to_y: 1 }), /outside viewport/);
});
test("A-L24: a drag from or onto a protected field is dispatched like any other", async () => {
  const { browser, sent } = fixture(true);
  await inputHostTab(browser, tab, { kind: "drag", x: 10, y: 10, to_x: 200, to_y: 10 });
  assert(sent.filter(x => x.method === "Input.dispatchMouseEvent").length >= 3);
  assert(!sent.some(x => x.method === "Runtime.evaluate"), "no page read before a drag");
});

test('MP-11 ordinary page input applies a retired visible crop guard before any press',async()=>{
 const {inputHostTab}=await import('./kernel-browser-input.mjs');const sent=[];
 const connection={send:async(method,params)=>{sent.push(method);if(method==='Page.getFrameTree')return {frameTree:{frame:{loaderId:'d'}}};return {};}};
 const browser={resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{run:async(_c,_s,fn)=>fn()}};
 await assert.rejects(inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'click',x:12,y:700},{inputGuard:async()=>{throw Error('retired visible crop');}}),/retired visible crop/);
 assert(!sent.some(m=>m.startsWith('Input.')),'a retired crop must dispatch no button press');
});
