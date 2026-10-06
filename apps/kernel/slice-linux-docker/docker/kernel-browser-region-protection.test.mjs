import assert from "node:assert/strict";
import test from "node:test";
import { captureRegionMasks } from "./kernel-browser-region-protection.mjs";

function fixture() {
  const state = { x: 10, fail: false, calls: [], shadow: false };
  const connection = { async send(method) {
    state.calls.push(method);
    if (state.fail) throw new Error("metadata unavailable");
    if (method === "DOM.getDocument") return { root: { nodeId: 1, children: state.shadow ? [{ nodeId: 3, shadowRoots: [{}] }] : [] } };
    if (method === "DOM.querySelectorAll") return { nodeIds: [2] };
    assert.equal(method, "DOM.getBoxModel");
    return { model: { border: [state.x, 20, state.x + 30, 20, state.x + 30, 60, state.x, 60] } };
  } };
  return { state, connection };
}
test("region masks preserve stable protected field bounds using only CDP DOM metadata", async () => {
  const { state, connection } = fixture();
  const masks = await captureRegionMasks(connection, "session");
  assert.deepEqual(await masks.afterCapture(), [{ x: 10, y: 20, width: 30, height: 40 }]);
  assert(state.calls.every(method => method.startsWith("DOM.")));
});
for (const change of ["layout", "metadata"]) {
  test(`region masks cover the full frame after a ${change} change during capture`, async () => {
    const { state, connection } = fixture();
    const masks = await captureRegionMasks(connection, "session");
    if (change === "layout") state.x = 100;
    else state.fail = true;
    assert.deepEqual(await masks.afterCapture(), [{ x: 0, y: 0, width: 1280, height: 800 }]);
  });
}
test("region capture protects shadow hosts and refuses unavailable initial protection", async () => {
  const { state, connection } = fixture();
  state.shadow = true;
  const masks = await captureRegionMasks(connection, "session");
  assert.equal((await masks.afterCapture()).length, 2);
  state.fail = true;
  await assert.rejects(captureRegionMasks(connection, "session"), /unavailable/);
});
