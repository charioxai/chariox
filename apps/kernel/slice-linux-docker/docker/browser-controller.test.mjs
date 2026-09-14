import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  BrowserControllerStdioServer,
  handleBrowserControllerRequest,
  parseMinimumDownloadFreeBytes,
} from "./browser-controller.mjs";
import {
  cancellationFixtureState,
  resetCancellationFixture,
  waitForCancellationFixture,
} from "./browser-import-cancellation-fixture.mjs";

test("controller derives download headroom from the shared slice setting", () => {
  assert.equal(parseMinimumDownloadFreeBytes(undefined), 256 * 1024 * 1024);
  assert.equal(parseMinimumDownloadFreeBytes("0"), 0);
  assert.equal(parseMinimumDownloadFreeBytes("2048"), 2048 * 1024 * 1024);
  assert.equal(parseMinimumDownloadFreeBytes("100000"), 100000 * 1024 * 1024);
  for (const value of ["-1", "1.5", "many", "8589934592"]) {
    assert.throws(() => parseMinimumDownloadFreeBytes(value), /CHARIOX_SLICE_MIN_FREE_MB/);
  }
});

test("controller health is request-correlated and process-owned", async () => {
  assert.deepEqual(await handleBrowserControllerRequest({ id: 7, method: "health" }, { processId: 41 }), {
    id: 7,
    ok: true,
    result: {
      state: "ready",
      process_id: 41,
      diagnostic_code: null,
    },
  });
});

test("controller rejects invalid and unknown requests", async () => {
  assert.equal((await handleBrowserControllerRequest({ id: 1, method: "missing" })).ok, false);
  assert.equal(
    (await handleBrowserControllerRequest({ method: "health" })).error.code,
    "invalid_request",
  );
});

test("stdio controller cancels the exact active cookie import and waits for rollback cleanup", async (t) => {
  resetCancellationFixture();
  const prior = process.env.CHARIOX_BROWSER_IMPORT_MODULE;
  process.env.CHARIOX_BROWSER_IMPORT_MODULE = new URL("./browser-import-cancellation-fixture.mjs",import.meta.url).href;
  t.after(() => {
    if (prior === undefined) delete process.env.CHARIOX_BROWSER_IMPORT_MODULE;
    else process.env.CHARIOX_BROWSER_IMPORT_MODULE = prior;
  });
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = readline.createInterface({input:output,crlfDelay:Infinity});
  const iterator = lines[Symbol.asyncIterator]();
  const server = new BrowserControllerStdioServer({input,output,browser:{}});
  const running = server.run();
  input.write(`${JSON.stringify({id:41,method:"browser.cookies.import",params:{
    binding:{request_id:"a".repeat(32)},domains:["example.test"],
  }})}\n`);
  await waitForCancellationFixture();
  input.write(`${JSON.stringify({id:42,method:"browser.cancel",params:{request_id:41}})}\n`);
  // The terminal rollback is emitted before cancellation is acknowledged.
  const terminal = JSON.parse((await iterator.next()).value);
  assert.equal(terminal.id,41);
  assert.equal(terminal.ok,true);
  assert.deepEqual(terminal.result,{status:"rolled_back",results:[]});
  const cancellation = JSON.parse((await iterator.next()).value);
  assert.deepEqual(cancellation,{id:42,ok:true,result:{accepted:true}});
  assert.deepEqual(cancellationFixtureState(),{
    started:true,mutations:0,rolledBack:true,cleanupComplete:true,
  });

  // A transaction that reached an applied terminal state cannot be reported
  // as cancelled merely because its registry entry was still draining.
  resetCancellationFixture();
  input.write(`${JSON.stringify({id:43,method:"browser.cookies.import",params:{
    binding:{request_id:"b".repeat(32)},domains:["example.test"],ignore_abort:true,
  }})}\n`);
  await waitForCancellationFixture();
  input.write(`${JSON.stringify({id:44,method:"browser.cancel",params:{request_id:43}})}\n`);
  assert.equal(JSON.parse((await iterator.next()).value).result.status,"applied");
  assert.deepEqual(JSON.parse((await iterator.next()).value),{id:44,ok:true,result:{accepted:false}});
  input.end();
  await running;
  lines.close();
});

test("controller delegates browser observations and closes CDP on shutdown", async () => {
  const calls = [];
  const browser = {
    reconcile: async (viewport) => {
      calls.push({ method: "reconcile", viewport });
      return { tabs: [], focused_target_id: null, viewport };
    },
    snapshot: async (request) => {
      calls.push({ method: "snapshot", request });
      return { target_id: request.target_id, document_id: request.document_id };
    },
    manageTab: async (request) => {
      calls.push({ method: "tab", request });
      return { target_id: request.target_id, action: request.action };
    },
    performAction: async (request) => {
      calls.push({ method: "action", request });
      return { action_kind: request.action.kind };
    },
    navigate: async (request) => {
      calls.push({ method: "navigate", request });
      return { url: request.url };
    },
    manageHistory: async (request) => {
      calls.push({ method: "history", request });
      return { action: request.action };
    },
    wait: async (request) => {
      calls.push({ method: "wait", request });
      return { kind: request.kind, ok: true };
    },
    handleDialog: async (request) => {
      calls.push({ method: "dialog", request });
      return { action: request.action };
    },
    configureDownloads: async (request) => {
      calls.push({ method: "downloads", request });
      return { enabled: true };
    },
    uploadFiles: async (request) => {
      calls.push({ method: "upload", request });
      return { file_count: request.file_paths.length };
    },
    setPermission: async (request) => {
      calls.push({ method: "permission", request });
      return { permission: request.permission, setting: request.setting };
    },
    pollEvents: (request) => {
      calls.push({ method: "events", request });
      return { events: [], next_cursor: request.cursor, replay_gap: false };
    },
    close: async () => calls.push({ method: "close" }),
  };
  const viewport = {
    css_width: 1280,
    css_height: 720,
    device_scale_factor: 1,
    desktop_pixel_width: 1280,
    desktop_pixel_height: 720,
  };

  const reconciled = await handleBrowserControllerRequest(
    { id: 1, method: "browser.reconcile", params: { viewport } },
    { browser },
  );
  const snapshot = await handleBrowserControllerRequest(
    {
      id: 2,
      method: "browser.snapshot",
      params: { target_id: "target-a", document_id: "loader-a" },
    },
    { browser },
  );
  const tab = await handleBrowserControllerRequest(
    {
      id: 12,
      method: "browser.tab",
      params: { target_id: "target-a", document_id: "loader-a", action: "activate" },
    },
    { browser },
  );
  const action = await handleBrowserControllerRequest(
    {
      id: 3,
      method: "browser.action",
      params: {
        target_id: "target-a",
        document_id: "loader-a",
        node_ref: "backend:103",
        action: { kind: "click" },
      },
    },
    { browser },
  );
  const dialog = await handleBrowserControllerRequest(
    {
      id: 4,
      method: "browser.dialog",
      params: {
        target_id: "target-a",
        document_id: "loader-a",
        action: "dismiss",
      },
    },
    { browser },
  );
  const navigation = await handleBrowserControllerRequest(
    {
      id: 10,
      method: "browser.navigate",
      params: {
        target_id: "target-a",
        document_id: "loader-a",
        url: "https://example.test/settings",
      },
    },
    { browser },
  );
  const history = await handleBrowserControllerRequest(
    {
      id: 13,
      method: "browser.history",
      params: {
        target_id: "target-a",
        document_id: "loader-b",
        action: "back",
      },
    },
    { browser },
  );
  const wait = await handleBrowserControllerRequest(
    {
      id: 11,
      method: "browser.wait",
      params: {
        target_id: "target-a",
        document_id: "loader-b",
        kind: "idle",
        timeout_ms: 500,
      },
    },
    { browser },
  );
  const downloads = await handleBrowserControllerRequest(
    {
      id: 5,
      method: "browser.downloads.configure",
      params: { target_id: "target-a", document_id: "loader-a" },
    },
    { browser },
  );
  const upload = await handleBrowserControllerRequest(
    {
      id: 6,
      method: "browser.upload",
      params: {
        target_id: "target-a",
        document_id: "loader-a",
        node_ref: "backend:103",
        file_paths: ["/safe/report.txt"],
      },
    },
    { browser },
  );
  const permission = await handleBrowserControllerRequest(
    {
      id: 7,
      method: "browser.permission",
      params: {
        target_id: "target-a",
        document_id: "loader-a",
        permission: "geolocation",
        setting: "denied",
      },
    },
    { browser },
  );
  const events = await handleBrowserControllerRequest(
    {
      id: 8,
      method: "browser.events.poll",
      params: { browser_generation: 1, cursor: 4, limit: 10 },
    },
    { browser },
  );
  const shutdown = await handleBrowserControllerRequest(
    { id: 9, method: "shutdown" },
    { browser },
  );

  assert.equal(reconciled.ok, true);
  assert.deepEqual(reconciled.result.viewport, viewport);
  assert.deepEqual(snapshot.result, {
    target_id: "target-a",
    document_id: "loader-a",
  });
  assert.deepEqual(tab.result, { target_id: "target-a", action: "activate" });
  assert.deepEqual(action.result, { action_kind: "click" });
  assert.deepEqual(dialog.result, { action: "dismiss" });
  assert.deepEqual(navigation.result, { url: "https://example.test/settings" });
  assert.deepEqual(history.result, { action: "back" });
  assert.deepEqual(wait.result, { kind: "idle", ok: true });
  assert.deepEqual(downloads.result, { enabled: true });
  assert.deepEqual(upload.result, { file_count: 1 });
  assert.deepEqual(permission.result, { permission: "geolocation", setting: "denied" });
  assert.deepEqual(events.result, { events: [], next_cursor: 4, replay_gap: false });
  assert.equal(shutdown.ok, true);
  assert.deepEqual(calls, [
    { method: "reconcile", viewport },
    {
      method: "snapshot",
      request: { target_id: "target-a", document_id: "loader-a" },
    },
    {
      method: "tab",
      request: { target_id: "target-a", document_id: "loader-a", action: "activate" },
    },
    {
      method: "action",
      request: {
        target_id: "target-a",
        document_id: "loader-a",
        node_ref: "backend:103",
        action: { kind: "click" },
      },
    },
    {
      method: "dialog",
      request: {
        target_id: "target-a",
        document_id: "loader-a",
        action: "dismiss",
      },
    },
    {
      method: "navigate",
      request: {
        target_id: "target-a",
        document_id: "loader-a",
        url: "https://example.test/settings",
      },
    },
    {
      method: "history",
      request: {
        target_id: "target-a",
        document_id: "loader-b",
        action: "back",
      },
    },
    {
      method: "wait",
      request: {
        target_id: "target-a",
        document_id: "loader-b",
        kind: "idle",
        timeout_ms: 500,
      },
    },
    {
      method: "downloads",
      request: { target_id: "target-a", document_id: "loader-a" },
    },
    {
      method: "upload",
      request: {
        target_id: "target-a",
        document_id: "loader-a",
        node_ref: "backend:103",
        file_paths: ["/safe/report.txt"],
      },
    },
    {
      method: "permission",
      request: {
        target_id: "target-a",
        document_id: "loader-a",
        permission: "geolocation",
        setting: "denied",
      },
    },
    {
      method: "events",
      request: { browser_generation: 1, cursor: 4, limit: 10 },
    },
    { method: "close" },
  ]);
});

test("stdio controller stays private to its owning process and shuts down cleanly", async (context) => {
  const child = spawn(
    process.execPath,
    [fileURLToPath(new URL("./browser-controller.mjs", import.meta.url)), "stdio"],
    { stdio: ["pipe", "pipe", "pipe"] },
  );
  const responses = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
  const iterator = responses[Symbol.asyncIterator]();
  context.after(() => {
    responses.close();
    child.stdin.destroy();
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
    }
  });

  child.stdin.write(`${JSON.stringify({ id: 1, method: "health" })}\n`);
  const health = JSON.parse((await iterator.next()).value);
  assert.equal(health.id, 1);
  assert.equal(health.ok, true);
  assert.equal(health.result.state, "ready");
  assert.equal(health.result.process_id, child.pid);

  child.stdin.write(`${JSON.stringify({ id: 2, method: "shutdown" })}\n`);
  const shutdown = JSON.parse((await iterator.next()).value);
  assert.deepEqual(shutdown, {
    id: 2,
    ok: true,
    result: {
      state: "stopped",
      process_id: null,
      diagnostic_code: null,
    },
  });
  child.stdin.end();

  const exit = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  assert.deepEqual(exit, { code: 0, signal: null });
});
