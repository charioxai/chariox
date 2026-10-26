import assert from "node:assert/strict";
import test from "node:test";
import { BrowserInputCapture } from "./browser-controller-input.mjs";

for (const outcome of ["complete", "cancel", "fail"]) {
  test(`MP-08/MP-10 background input restores emulation after ${outcome}`, async () => {
    const capture = new BrowserInputCapture();
    const calls = [];
    const connection = { async send(method, params, session) {
      calls.push({ method, params, session });
      return { result: { value: false } };
    } };
    const pending = capture.run(connection, "background", async () => {
      assert.equal(await capture.visibility(connection, "background"), false);
      assert.equal(await capture.visibility(connection, "foreground"), false);
      if (outcome !== "complete") throw new Error(outcome);
      return 42;
    });
    if (outcome === "complete") assert.equal(await pending, 42);
    else await assert.rejects(pending, new RegExp(outcome));
    assert.deepEqual(calls.filter(c => c.method.startsWith("Emulation.")), [
      { method: "Emulation.setFocusEmulationEnabled", params: { enabled: true }, session: "background" },
      { method: "Emulation.setFocusEmulationEnabled", params: { enabled: false }, session: "background" },
    ]);
    assert.equal(capture.visibilityBySession.size, 0);
    assert.ok(calls.every(c => c.method !== "Page.bringToFront"));
  });
}

test("MP-08/MP-10 foreground input needs no capture", async () => {
  const capture = new BrowserInputCapture();
  const calls = [];
  const connection = { async send(method) { calls.push(method); return { result: { value: true } }; } };
  assert.equal(await capture.run(connection, "foreground", async () => 42), 42);
  assert.deepEqual(calls, ["Runtime.evaluate"]);
});

test("MP-08/MP-10 dialog input releases capture without waiting for the dialog renderer", async () => {
  const capture = new BrowserInputCapture();
  const released = Promise.withResolvers();
  const connection = { async send(method, params) {
    if (method === "Emulation.setFocusEmulationEnabled" && !params.enabled) await released.promise;
    return { result: { value: false } };
  } };
  const result = await capture.run(connection, "background", async () => ({ dialogOpened: true }));
  assert.equal(result.dialogOpened, true);
  assert.equal(await capture.visibility(connection, "background"), false);
  released.resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(capture.visibilityBySession.size, 0);
});

test("MP-08/MP-10 pending dialog restore cannot clear a later input visibility capture", async () => {
  const capture = new BrowserInputCapture();
  const releaseDialog = Promise.withResolvers();
  let restoring = 0;
  let evaluations = 0;
  const connection = { async send(method, params) {
    if (method === "Emulation.setFocusEmulationEnabled" && !params.enabled && ++restoring === 1) {
      await releaseDialog.promise;
    }
    return { result: { value: method === "Runtime.evaluate" && ++evaluations > 1 } };
  } };
  // First physical visibility is hidden; later CDP evaluates emulated visibility.
  await capture.run(connection, "background", async () => ({ dialogOpened: true }));
  await capture.run(connection, "background", async () => {
    releaseDialog.resolve();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(await capture.visibility(connection, "background"), false);
  });
  assert.equal(capture.visibilityBySession.size, 0);
});

test('MD-DISPLAY hidden display hold retains physical visibility and drains input before restore',async()=>{
 const capture=new BrowserInputCapture(),calls=[];
 const connection={send:async(method,params)=>{calls.push({method,params});return {result:{value:false}}}};
 const first=await capture.hold(connection,'hidden'),second=await capture.hold(connection,'hidden');
 assert.equal(await capture.visibility(connection,'hidden'),false);
 let releaseInput,entered;const held=new Promise(r=>releaseInput=r),started=new Promise(r=>entered=r);
 const input=capture.run(connection,'hidden',async()=>{entered();await held});await started;
 await first();let restored=false;const closing=second().then(()=>restored=true);
 await new Promise(r=>setImmediate(r));assert.equal(restored,false);
 releaseInput();await input;await closing;
 assert.deepEqual(calls.filter(x=>x.method.startsWith('Emulation.')).map(x=>x.params.enabled),[true,false]);
 assert.equal(capture.visibilityBySession.size,0);assert.equal(capture.displayLeases.size,0);
 assert(!calls.some(x=>x.method==='Page.bringToFront'));
});

test('MP-08/MP-10: a display hold acquired while visible still keeps a later hidden renderer awake',async()=>{
  const capture=new BrowserInputCapture(),calls=[];let physicallyVisible=true,emulated=false;
  const connection={async send(method,params){calls.push(method);if(method==='Emulation.setFocusEmulationEnabled')emulated=params.enabled;return {result:{value:physicallyVisible||emulated}};}};
  const release=await capture.hold(connection,'display');physicallyVisible=false;
  try{await capture.run(connection,'display',async()=>assert.equal(emulated,true,'the logical lease cannot bypass a renderer wake-up'));}
  finally{await release()}
  assert.equal(emulated,false);assert.equal(capture.displayLeases.size,0);assert(!calls.includes('Page.bringToFront'));
});

// MP-08/MP-10: a viewer attaching to a page whose renderer is parsing (a 16 MB document) must not wait on a renderer read.
test('MP-08/MP-10: a display hold acquired while the renderer is busy assumes the page hidden and keeps it awake',async()=>{
  const capture=new BrowserInputCapture(),calls=[];
  const connection={async send(method,params,session,options){
    calls.push({method,params,options});
    if(method==='Runtime.evaluate')throw Object.assign(Error(`browser CDP method ${method} timed out after ${options?.timeoutMs}ms`),{code:'browser_cdp_timeout'});
    return {};
  }};
  const release=await capture.hold(connection,'display');
  assert(calls.find(c=>c.method==='Runtime.evaluate').options.timeoutMs<=2000,'the visibility read is short');
  assert.deepEqual(calls.filter(c=>c.method.startsWith('Emulation.')).map(c=>c.params.enabled),[true],'the wake-up is acquired');
  await release();
  assert.deepEqual(calls.filter(c=>c.method.startsWith('Emulation.')).map(c=>c.params.enabled),[true,false]);
  assert.equal(capture.displayLeases.size,0);assert.equal(capture.visibilityBySession.size,0);
});

test('MP-08/MP-10: an input run (not a display hold) still reports a busy renderer',async()=>{
  const capture=new BrowserInputCapture();
  const connection={async send(){throw Object.assign(Error('timed out'),{code:'browser_cdp_timeout'})}};
  await assert.rejects(capture.run(connection,'s',async()=>1),{code:'browser_cdp_timeout'});
});

// MP-11: physical focus sampling neither overlaps input nor releases a live display.
test('MP-11 physical focus sampling restores emulation after a refusal and orders later input',async()=>{
 const capture=new BrowserInputCapture();let emulated=false,finish,entered;
 const gate=new Promise(r=>finish=r),started=new Promise(r=>entered=r),calls=[];
 const connection={send:async(method,p)=>{if(method==='Emulation.setFocusEmulationEnabled')emulated=p.enabled;return{result:{value:true}}}};
 const release=await capture.hold(connection,'s');
 const read=capture.physicalFocus(connection,'s',async()=>{assert.equal(emulated,false);calls.push('read');entered();await gate;throw Error('refused')});
 await started;
 const input=capture.run(connection,'s',async()=>{assert.equal(emulated,true);calls.push('input')});
 await new Promise(r=>setImmediate(r));assert.deepEqual(calls,['read']);finish();
 await assert.rejects(read,/refused/);await input;await release();
 assert.deepEqual(calls,['read','input']);assert.equal(emulated,false);assert.equal(capture.displayLeases.size,0);
});
test('MP-11 a display closing during physical focus sampling stays closed',async()=>{
 const capture=new BrowserInputCapture();let emulated=false,finish,entered;
 const gate=new Promise(r=>finish=r),started=new Promise(r=>entered=r);
 const connection={send:async(method,p)=>{if(method==='Emulation.setFocusEmulationEnabled')emulated=p.enabled;return{result:{value:true}}}};
 const release=await capture.hold(connection,'s');
 const read=capture.physicalFocus(connection,'s',async()=>{entered();await gate;return false});await started;
 const close=release();finish();assert.equal(await read,false);await close;
 assert.equal(emulated,false);assert.equal(capture.visibilityBySession.size,0);
});

// MP-08/MP-10/MP-11: omnibox typing must not synthesize a page focus event
// after each character; observations also preserve that physical UI focus.
test('MP-11 native Chrome focus stays physical across typing and screenshots until page input',async()=>{
 const capture=new BrowserInputCapture(),changes=[];let emulated=false;
 const connection={send:async(method,p)=>{if(method==='Emulation.setFocusEmulationEnabled'){emulated=p.enabled;changes.push(p.enabled)}return{result:{value:true}}}};
 const release=await capture.hold(connection,'s');
 try{
  for(let i=0;i<5;i++){assert.equal(await capture.physicalFocus(connection,'s',async()=>false),false);assert.equal(emulated,false,'the page stays blurred while Chrome owns the keys')}
  await capture.run(connection,'s',async()=>assert.equal(emulated,false),{resumeFocus:false});
  assert.deepEqual(changes,[true,false],'observation and typing do not flap page focus');
  await capture.run(connection,'s',async()=>assert.equal(emulated,true,'page input resumes the renderer lease'));
  assert.deepEqual(changes,[true,false,true]);
 }finally{await release()}
 assert.equal(emulated,false);assert.equal(capture.visibilityBySession.size,0);
});
