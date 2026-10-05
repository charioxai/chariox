// MP-08/MP-11: dispatch waits for the live mirror guard after focus capture.
import test from 'node:test';
import assert from 'node:assert/strict';
import { inputHostTab } from './kernel-browser-input.mjs';
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
   const events=[];const connection={async send(method,params){if(method==='Page.getFrameTree')return {frameTree:{frame:{loaderId:'d'}}};if(method==='Input.dispatchKeyEvent'){events.push(params);return {}};throw Error(`unexpected ${method}`)}};
   const browser={async resolvePageTarget(){return {connection,sessionId:'s'}},inputCapture:{run(_c,_s,fn){return fn()}}};
   await inputHostTab(browser,{target_id:'t',document_id:'d'},{kind:'key',key});
   assert.deepEqual(events.map(e=>e.windowsVirtualKeyCode),[code,code]);assert.deepEqual(events.map(e=>e.type),['keyDown','keyUp']);
 }
});
