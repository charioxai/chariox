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


test('MP-08/MP-11: native navigation and editing keys carry their Chromium virtual key codes',async()=>{
 const codes={Tab:9,Enter:13,Escape:27,Backspace:8,Delete:46,ArrowLeft:37,ArrowRight:39,ArrowUp:38,ArrowDown:40,Home:36,End:35};
 for(const [key,code]of Object.entries(codes)){
   const events=[];const connection={async send(method,params){if(method==='Page.getFrameTree')return {frameTree:{frame:{loaderId:'d'}}};if(method==='Input.dispatchKeyEvent'){events.push(params);return {}};if(method==='Page.createIsolatedWorld')return {executionContextId:7};if(method==='Runtime.evaluate')return {result:{value:false}};throw Error(`unexpected ${method}`)}};
   const browser={async resolvePageTarget(){return {connection,sessionId:'s'}},inputCapture:{run(_c,_s,fn){return fn()}}};
   await inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'key',key});
   assert.deepEqual(events.map(e=>e.windowsVirtualKeyCode),[code,code]);assert.deepEqual(events.map(e=>e.type),['keyDown','keyUp']);
 }
});
