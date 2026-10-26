import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import { BrowserControllerError } from "./browser-controller-cdp.mjs";

const deferred = () => Promise.withResolvers();

function startServer(t, browser, release = () => {}) {
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = readline.createInterface({ input: output, crlfDelay: Infinity });
  const waiters = [];
  const buffered = [];
  lines.on("line", line => {
    const response = JSON.parse(line);
    const index = waiters.findIndex(({ matches }) => matches(response));
    if (index < 0) buffered.push(response);
    else waiters.splice(index, 1)[0].result.resolve(response);
  });
  const running = new BrowserControllerStdioServer({ input, output, browser }).run();
  t.after(async () => { release(); input.end(); await running; lines.close(); output.end(); });
  return {
    request(id, method, params = {}, matches = response => response.id === id) {
      const index = buffered.findIndex(matches);
      if (index >= 0) return Promise.resolve(buffered.splice(index, 1)[0]);
      const result = deferred();
      waiters.push({ matches, result });
      input.write(`${JSON.stringify({ id, method, params })}\n`);
      return result.promise;
    },
  };
}

async function assertNotStarted(started, message) {
  assert.equal(await Promise.race([
    started.promise.then(() => true),
    new Promise(resolve => setTimeout(() => resolve(false), 30)),
  ]), false, message);
}

test("stdio overlaps independent-tab mutations", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred();
  const server = startServer(t, { async performAction(params) {
    if (params.target_id === "first") { started.resolve(); await release.promise; }
    return { target_id: params.target_id };
  } }, () => release.resolve());
  const first = server.request(1, "browser.action", { target_id: "first" });
  await started.promise;
  assert.deepEqual((await server.request(2, "browser.action", { target_id: "second" })).result, { target_id: "second" });
  release.resolve();
  assert.deepEqual((await first).result, { target_id: "first" });
});

test("stdio serializes same-tab mutations", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred(), secondStarted = deferred();
  let calls = 0;
  const server = startServer(t, { async performAction() {
    calls += 1;
    if (calls === 1) { started.resolve(); await release.promise; }
    else secondStarted.resolve();
    return { call: calls };
  } }, () => release.resolve());
  const first = server.request(1, "browser.action", { target_id: "same" });
  await started.promise;
  const second = server.request(2, "browser.action", { target_id: "same" });
  await assertNotStarted(secondStarted, "same-tab work must wait");
  assert.equal(calls, 1);
  release.resolve();
  assert.equal((await first).ok, true);
  assert.deepEqual((await second).result, { call: 2 });
});

// MP-08/MP-10/MP-11: admission is not promotion. A failed predecessor must
// release its target exactly once without draining another tab's mutations.
test("MP-08/MP-10/MP-11 failed predecessor promotes queued work exactly once", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred(), promoted = deferred();
  const effects = [];
  const server = startServer(t, { async performAction(params) {
    if (params.tag === "first") {
      started.resolve(); await release.promise;
      throw new BrowserControllerError("browser_action_cancelled", "predecessor cancelled");
    }
    effects.push(params.tag);
    if (params.tag === "queued") promoted.resolve();
    return { tag: params.tag };
  } }, () => release.resolve());
  const first = server.request(1, "browser.action", { target_id: "same", tag: "first" });
  await started.promise;
  const queued = server.request(2, "browser.action", { target_id: "same", tag: "queued" });
  assert.equal((await server.request(3, "browser.action", { target_id: "other", tag: "independent" })).ok, true);
  await assertNotStarted(promoted, "queued work must not dispatch before predecessor settles");
  release.resolve();
  assert.equal((await first).error.code, "browser_action_cancelled");
  await promoted.promise;
  assert.deepEqual((await queued).result, { tag: "queued" });
  assert.equal((await server.request(4, "browser.action", { target_id: "same", tag: "later" })).ok, true);
  assert.deepEqual(effects, ["independent", "queued", "later"]);
});

test("stdio cancellation acknowledges held tab while independent tab finishes", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred();
  const server = startServer(t, { async performAction(params, { signal }) {
    if (params.target_id === "first") {
      started.resolve(); await release.promise;
      if (signal.aborted) throw new BrowserControllerError("browser_action_cancelled", "browser action was cancelled");
    }
    return { target_id: params.target_id };
  } }, () => release.resolve());
  const first = server.request(1, "browser.action", { target_id: "first" });
  await started.promise;
  assert.deepEqual((await server.request(2, "browser.cancel", { request_id: 1 })).result, { accepted: true });
  assert.deepEqual((await server.request(3, "browser.action", { target_id: "second" })).result, { target_id: "second" });
  await assertNotStarted({ promise: first }, "held action must await its browser call");
  release.resolve();
  assert.equal((await first).error.code, "browser_action_cancelled");
});

test("stdio cancellation prevents queued same-tab physical dispatch", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred();
  let queuedCalls = 0;
  const server = startServer(t, { async performAction(params) {
    if (params.tag === "first") { started.resolve(); await release.promise; }
    else queuedCalls += 1;
    return { tag: params.tag };
  } }, () => release.resolve());
  const first = server.request(1, "browser.action", { target_id: "same", tag: "first" });
  await started.promise;
  const queued = server.request(2, "browser.action", { target_id: "same", tag: "queued" });
  assert.deepEqual((await server.request(3, "browser.cancel", { request_id: 2 })).result, { accepted: true });
  await assertNotStarted({ promise: queued }, "queued cancellation must retain target ordering");
  release.resolve();
  assert.equal((await first).ok, true);
  assert.equal((await queued).error.code, "browser_action_cancelled");
  assert.equal(queuedCalls, 0);
});

test("stdio duplicate IDs and 64 admission bound span all target lanes", { timeout: 3000 }, async t => {
  const started = deferred(), allStarted = deferred(), release = deferred();
  let calls = 0;
  const server = startServer(t, { async performAction(params) {
    calls += 1;
    if (calls === 1) started.resolve();
    if (calls === 64) allStarted.resolve();
    await release.promise;
    return { target_id: params.target_id };
  } }, () => release.resolve());
  const admitted = [server.request(1, "browser.action", { target_id: "tab-1" }, response => response.id === 1 && response.ok)];
  await started.promise;
  assert.equal((await server.request(1, "browser.action", { target_id: "duplicate" }, response => response.id === 1 && !response.ok)).error.code, "controller_busy");
  assert.equal(calls, 1);
  for (let id = 2; id <= 64; id += 1) admitted.push(server.request(id, "browser.action", { target_id: `tab-${id}` }));
  await allStarted.promise;
  assert.equal(calls, 64);
  assert.equal((await server.request(65, "browser.action", { target_id: "overflow" })).error.code, "controller_busy");
  assert.equal(calls, 64);
  release.resolve();
  assert.ok((await Promise.all(admitted)).every(response => response.ok));
});

test("stdio global barriers drain active and queued reads and mutations", { timeout: 3000 }, async t => {
  const mutationStarted = deferred(), releaseMutation = deferred(), readStarted = deferred(), releaseRead = deferred();
  const queuedStarted = deferred(), releaseQueued = deferred(), barrierStarted = deferred(), releaseBarrier = deferred(), laterStarted = deferred();
  const release = () => { releaseMutation.resolve(); releaseRead.resolve(); releaseQueued.resolve(); releaseBarrier.resolve(); };
  const server = startServer(t, {
    async performAction(params) {
      if (params.tag === "first") { mutationStarted.resolve(); await releaseMutation.promise; }
      else if (params.tag === "queued") { queuedStarted.resolve(); await releaseQueued.promise; }
      else laterStarted.resolve();
      return { tag: params.tag };
    },
    async snapshot(params) { readStarted.resolve(); await releaseRead.promise; return { target_id: params.target_id }; },
    async reconcile(viewport) {
      barrierStarted.resolve(); await releaseBarrier.promise;
      return { tabs: [], focused_target_id: null, viewport,
        resource_inventory: { browser_ids: ["browser-1"], profile_ids: ["profile-1"] } };
    },
  }, release);
  const mutation = server.request(1, "browser.action", { target_id: "tab-one", tag: "first" });
  const read = server.request(2, "browser.snapshot", { target_id: "tab-two" });
  await Promise.all([mutationStarted.promise, readStarted.promise]);
  const queued = server.request(3, "browser.action", { target_id: "tab-one", tag: "queued" });
  const barrier = server.request(4, "browser.reconcile", { viewport: { css_width: 1 } });
  const later = server.request(5, "browser.action", { target_id: "tab-three", tag: "later" });
  releaseMutation.resolve(); await queuedStarted.promise;
  await assertNotStarted(barrierStarted, "barrier must drain queued mutations");
  releaseRead.resolve();
  await assertNotStarted(barrierStarted, "barrier must drain active mutations");
  releaseQueued.resolve(); await barrierStarted.promise;
  await assertNotStarted(laterStarted, "later work must wait behind the barrier");
  releaseBarrier.resolve();
  assert.ok((await Promise.all([mutation, read, queued, barrier, later])).every(response => response.ok));
});

// MP-08/MP-10: normal read preflight must not reserve every target.
test("unchanged-viewport reconciliation does not block independent input", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred();
  const server = startServer(t, {
    canReconcileConcurrently: () => true,
    async performAction(params) {
      if (params.target_id === "held") { started.resolve(); await release.promise; }
      return { target_id: params.target_id };
    },
    async reconcile(viewport) { return { tabs: [], viewport, focused_target_id: null,
      resource_inventory: { browser_ids: ["browser-1"], profile_ids: ["profile-1"] } }; },
  }, () => release.resolve());
  const held = server.request(1, "browser.action", { target_id: "held" });
  await started.promise;
  const read = server.request(2, "browser.reconcile", { viewport: { css_width: 1280 } });
  const other = server.request(3, "browser.action", { target_id: "other" });
  const completed = await Promise.race([Promise.all([read, other]).then(() => true),
    new Promise(resolve => setTimeout(() => resolve(false), 100))]);
  release.resolve();
  await Promise.all([held, read, other]);
  assert.equal(completed, true, "MP-08/MP-10 read preflight blocked an independent target");
});

test("idle App drain and reply overlap a held global browser barrier", { timeout: 3000 }, async t => {
  const started = deferred(), release = deferred(), drainStarted = deferred(), enqueue = deferred();
  const server = startServer(t, {
    async manageTab() { started.resolve(); await release.promise; return {}; },
    appTabs: {
      async takeCalls() { drainStarted.resolve(); await enqueue.promise; return { calls: [{ call_id: "idle" }] }; },
      async respond() { return { delivered: true }; },
    },
  }, () => { release.resolve(); enqueue.resolve(); });
  const barrier = server.request(1, "browser.tab", {});
  await started.promise;
  const drain = server.request(2, "browser.app.calls");
  await drainStarted.promise;
  assert.deepEqual((await server.request(3, "browser.app.respond", {})).result, { delivered: true });
  enqueue.resolve();
  assert.equal((await drain).result.calls[0].call_id, "idle");
  release.resolve();
  assert.equal((await barrier).ok, true);
});

test("an idle App drain does not block lifecycle barriers", { timeout: 3000 }, async t => {
  const started = deferred(), enqueue = deferred();
  const server = startServer(t, {
    async manageTab() { return { closed: true }; },
    appTabs: { async takeCalls() { started.resolve(); await enqueue.promise; return { calls: [] }; } },
  }, () => enqueue.resolve());
  const drain = server.request(1, "browser.app.calls");
  await started.promise;
  assert.deepEqual((await server.request(2, "browser.tab", {})).result, { closed: true });
  enqueue.resolve();
  assert.deepEqual((await drain).result.calls, []);
});
