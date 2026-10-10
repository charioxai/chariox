// MP-08/MP-11: authority-sensitive native dispatch, without page values.
import test from "node:test";
import assert from "node:assert/strict";
import { runInNewContext } from "node:vm";
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
test('MP-08/MP-11: admitted same-origin mirror frames inspect the live leaf and fail closed for protected or opaque targets',async()=>{
  const element=(fields={})=>({tagName:'INPUT',closest:()=>null,...fields});
  for(const [name,leaf,protectedTarget] of [
    ['ordinary',element(),false],['password',element({type:'password'}),true],
    ['uppercase OTP',element({autocomplete:'ONE-TIME-CODE'}),true],
    ['payment',element({autocomplete:'CC-NUMBER'}),true],
    ['protected ancestor',element({closest:()=>({})}),true],
    ['shadow password',element({shadowRoot:{activeElement:element({type:'password'})}}),true],
    ['opaque',null,true],
  ]) for(const mode of ['direct','fallback','observed']) {
    const mirror=mode!=='direct',observed=mode==='observed';
    const {browser,sent}=fixture();const {connection}=await browser.resolvePageTarget();const send=connection.send;
    connection.send=async(method,params)=>method==='Runtime.evaluate'
      ? {result:{value:runInNewContext(params.expression,{document:{activeElement:element({tagName:'IFRAME',contentDocument:leaf&&{activeElement:leaf}})}})}}
      : send(method,params);
    const input={kind:'text',text:'fixture'};
    const pending=inputHostTab(browser,tab,mirror?{kind:'mirror'}:input,
      mirror?{resolveMirror:async()=>({input,guard:async()=>{},observedFrameInput:observed})}:{});
    if(protectedTarget||!observed) {await assert.rejects(pending,{code:'user_domain_sensitive_requires_focus'},name);assert.equal(sent.filter(x=>x.method.startsWith('Input.')).length,0);}
    else {await pending;assert.equal(sent.filter(x=>x.method==='Input.insertText').length,1);}
  }
});
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
test("MP-08/MP-11: focused paired Tab still checks revocation on release",async()=>{
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
  assert.equal(sent.filter(call=>call.method==='Input.dispatchKeyEvent').length,1);
});

// MP-11: the public Rust key is an arbitrary string; the advertised MCP enum is not authority.
test("MP-11: protected targets refuse every text-producing input in both grant modes", async () => {
  for (const retained of [false, true]) for (const input of [
    {kind:'text',text:'fixture'}, ...['a','é','😀',' ', 'Enter','Space'].map(key=>({kind:'key',key})),
  ]) {
    const {browser,sent}=fixture(true);
    await assert.rejects(inputHostTab(browser,tab,input,{retained}), {code:'user_domain_sensitive_requires_focus'});
    assert.equal(sent.filter(({method})=>method.startsWith('Input.')).length,0);
  }
});
test("MP-08/MP-11: protected targets retain non-text navigation", async () => {
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
  assert.equal(dispatches,2);assert.equal(guards,2,'MP-08: release remains paired with press');
});

test('MP-11: IME shares the text protection preflight before the final mirror guard',async()=>{
 for(const protectedTarget of [false,true]) {
  const {browser,sent}=fixture(protectedTarget);let guarded=false;
  const pending=inputHostTab(browser,tab,{kind:'mirror'},{resolveMirror:async()=>({
   observedFrameInput:true,
   guard:async()=>{assert(sent.some(x=>x.method==='Runtime.evaluate'));guarded=true;},
   perform:send=>send('Input.imeSetComposition',{text:'文',selectionStart:1,selectionEnd:1}),
  })});
  if(protectedTarget){await assert.rejects(pending,{code:'user_domain_sensitive_requires_focus'});assert(!guarded);}
  else {await pending;assert(guarded);}
  assert.equal(sent.filter(x=>x.method.startsWith('Input.')).length,protectedTarget?0:1);
 }
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
  const result=await inputHostTab(browser,tab,{kind:'scroll',x:10,y:10,delta_x:0,delta_y:120},{asyncScroll:true,onDispatch:()=>dispatched++});
  assert.ok(result.ack instanceof Promise,'lane work returns before the renderer ack');
  assert.equal(dispatched,1);
  const methods=sent.map(x=>x.method);assert.ok(methods.indexOf('Page.getFrameTree')<methods.indexOf('Input.dispatchMouseEvent'),'document fence precedes dispatch');
  release();await result.ack;
  assert.equal(await inputHostTab(browser,tab,{kind:'scroll',x:10,y:10,delta_x:0,delta_y:120}),undefined,'synchronous callers still await the ack');
});
// MP-08/MP-10: wheel notches use the owned display after the same fences;
// precise deltas and unavailable native input keep CDP or fail closed.
test('MP-08/MP-10 notch wheel input routes to the owned display after the document fence',async()=>{
  const {browser,sent}=fixture();const wheels=[];let dispatched=0;
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:-240},{nativeWheel:(...a)=>{wheels.push(a);return true},onDispatch:()=>dispatched++});
  assert.deepEqual(wheels,[[10,20,0,-2]]);assert.equal(dispatched,1);
  assert.ok(sent.some(x=>x.method==='Page.getFrameTree'),'document fence ran');assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,0);
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:37},{nativeWheel:()=>assert.fail('precise deltas stay on CDP')});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,1);
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{nativeWheel:()=>false});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,2,'MP-11: refused native input falls back to CDP');
});

// MP-08/MP-10: line-sized wheel deltas from real viewers animate natively.
test('MP-08/MP-10 viewer wheel deltas map to native notches; fine trackpad deltas stay precise',async()=>{
 const {notches}=await import('./kernel-browser-input.mjs');
 assert.deepEqual([120,-240,100,-100,200,53,-80,0].map(notches),[1,-2,1,-1,2,1,-1,0]);
 for(const delta of [1,-12,37,49])assert.equal(notches(delta),null,String(delta));
});
// MP-08/MP-10: viewer clicks on the owned display use XTest after the fence.
test('MP-08/MP-10 viewer click routes to the owned display after the document fence; refusal falls back to CDP',async()=>{
  const {browser,sent}=fixture();const clicks=[];let dispatched=0;
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:(...a)=>{clicks.push(a);return true},onDispatch:()=>dispatched++});
  assert.deepEqual(clicks,[[10,20]]);assert.equal(dispatched,1);
  assert.ok(sent.some(x=>x.method==='Page.getFrameTree'),'document fence ran');assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,0);
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:()=>false});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,2,'MP-11: refused native click falls back to CDP press/release');
  // MP-11 (review #893 P2): the worker's asynchronous refusal also falls back.
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:async()=>false,onDispatch:()=>dispatched++});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,4,'MP-11: a covered owned window refuses; CDP reaches the renderer');
  // MP-11 (review #893 @8067044d1 P2): a native click/wheel whose reply was
  // lost may have been dispatched: it is never replayed via CDP.
  const uncertain=async()=>{throw Object.assign(Error('MP-11: native input outcome uncertain'),{code:'native_input_uncertain'})};
  const before=dispatched;
  await inputHostTab(browser,tab,{kind:'click',x:10,y:20},{nativeClick:uncertain,onDispatch:()=>dispatched++});
  await inputHostTab(browser,tab,{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120},{nativeWheel:uncertain,onDispatch:()=>dispatched++});
  assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,4,'MP-11: no CDP replay of an uncertain native action');
  assert.equal(dispatched,before+2,'the uncertain action counts as dispatched once');
});
// MP-08/MP-10: viewer keys on the owned display use XTest after the document,
// text-target and focus fences; anything else stays on CDP.
test('MP-08/MP-10 viewer keys route to the owned display only for a focused, non-sensitive page',async()=>{
  const run=async({sensitive=false,focused=true,native=()=>true,key='a'})=>{
    const sent=[],keys=[];let dispatched=0;
    const connection={send:async(method,params)=>{sent.push({method,params});
      if(method==='Page.getFrameTree')return {frameTree:{frame:{id:'f',loaderId:'doc'}}};
      if(method==='Page.createIsolatedWorld')return {executionContextId:7};
      if(method==='Runtime.evaluate')return {result:{value:params.expression==='document.hasFocus()'?focused:params.expression.includes("? 'sensitive'")?(sensitive?'sensitive':focused):sensitive}};
      return {};}};
    const browser={resolvePageTarget:async()=>({connection,sessionId:'s'}),inputCapture:{run:async(_c,_s,fn)=>fn()}};
    const result=await inputHostTab(browser,tab,{kind:'key',key},{nativeKey:(...a)=>{keys.push(a);return native(...a)},onDispatch:()=>dispatched++}).then(()=>'ok',e=>e.code??e.message);
    return {result,keys,dispatched,cdp:sent.filter(x=>x.method==='Input.dispatchKeyEvent').length};
  };
  assert.deepEqual(await run({}),{result:'ok',keys:[[97,false]],dispatched:1,cdp:0});
  assert.deepEqual(await run({key:'Shift+Tab'}),{result:'ok',keys:[[0xff09,true]],dispatched:1,cdp:0});
  assert.deepEqual(await run({key:'Enter'}),{result:'ok',keys:[[0xff0d,false]],dispatched:1,cdp:0});
  const refused=await run({sensitive:true});
  assert.deepEqual([refused.keys.length,refused.cdp],[0,0],'MP-11: a sensitive text target is refused before any dispatch');
  assert.match(String(refused.result),/sensitive/);
  assert.deepEqual(await run({focused:false}),{result:'ok',keys:[],dispatched:2,cdp:2},'unfocused page (browser UI focus) stays on CDP');
  assert.deepEqual(await run({key:'é'}),{result:'ok',keys:[],dispatched:2,cdp:2},'non-ASCII text stays on CDP');
  assert.deepEqual(await run({native:()=>false}),{result:'ok',keys:[[97,false]],dispatched:2,cdp:2},'MP-11: refused native key falls back to CDP');
});
