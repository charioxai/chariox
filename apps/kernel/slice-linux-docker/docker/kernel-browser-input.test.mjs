// MP-08/MP-11: authority-sensitive native dispatch, without page values.
import test from "node:test";
import assert from "node:assert/strict";
import { inputHostTab } from "./kernel-browser-input.mjs";

function fixture() {
  const sent = [];
  const connection = { send: async (method, params) => {
    sent.push({ method, params });
    if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "f", loaderId: "doc" } } };
    if (method === "Page.createIsolatedWorld") return { executionContextId: 7 };
    return {};
  } };
  return { sent, browser: { resolvePageTarget: async () => ({ connection, sessionId: "s" }), inputCapture: { run: async (_c, _s, fn) => fn() } } };
}
const tab = { target_id: "t", document_id: "doc" };
test("MP-11: every retained key, text and click refuses before native dispatch", async () => {
  const {browser,sent}=fixture(false);
  for(const input of [{kind:'click',x:1,y:2},{kind:'text',text:'fixture'},
    ...['Tab','Shift+Tab','Enter','Space','Delete','Backspace','ArrowLeft','Home','a','Escape','F1','Control+a'].map(key=>({kind:'key',key}))]) {
    sent.length=0;
    await assert.rejects(inputHostTab(browser,tab,input,{retained:true}),{code:'sensitive_requires_focus'});
    assert(!sent.some(({method})=>method.startsWith('Input.')));
    assert(!sent.some(({method})=>method.startsWith('DOMDebugger.')));
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
