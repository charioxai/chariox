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
