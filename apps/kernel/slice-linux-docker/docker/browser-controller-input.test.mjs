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
