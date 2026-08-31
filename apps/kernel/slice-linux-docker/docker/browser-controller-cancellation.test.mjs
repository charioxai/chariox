import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import test from "node:test";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

test("stdio cancellation interrupts a waiting action without losing the browser", async (t) => {
  let blocked = true;
  let holdMouseMove = false;
  let presses = 0;
  let releases = 0;
  let holdMousePress = false;
  const mousePressed = Promise.withResolvers();
  const releaseMousePress = Promise.withResolvers();
  const mouseMoved = Promise.withResolvers();
  const releaseMouseMove = Promise.withResolvers();
  const observed = Promise.withResolvers();
  const chromium = {
    isOpen: () => true,
    close: async () => {},
    async send(method, params) {
      switch (method) {
        case "Target.getTargets": return { targetInfos: [{ type: "page", targetId: "page" }] };
        case "Target.attachToTarget": return { sessionId: "cdp" };
        case "Page.getFrameTree": return { frameTree: { frame: { loaderId: "document" } } };
        case "DOM.resolveNode": return { object: { objectId: "button" } };
        case "Runtime.callFunctionOn":
          observed.resolve();
          return { result: { value: blocked ? { state: "disabled" } : { state: "ready", x: 10, y: 10, width: 20, height: 20 } } };
        case "Target.setDiscoverTargets":
        case "Page.enable":
        case "Page.setLifecycleEventsEnabled":
        case "Runtime.enable":
        case "Network.enable":
        case "Inspector.enable":
        case "Runtime.releaseObject": return {};
        case "Input.dispatchMouseEvent":
          if (params.type === "mousePressed") presses += 1;
          if (params.type === "mouseReleased") releases += 1;
          if (holdMousePress && params.type === "mousePressed") {
            mousePressed.resolve();
            await releaseMousePress.promise;
          }
          if (holdMouseMove && params.type === "mouseMoved") {
            mouseMoved.resolve();
            await releaseMouseMove.promise;
          }
          return {};
        default: throw new Error(`Unexpected external browser command: ${method}`);
      }
    },
  };
  const input = new PassThrough();
  const output = new PassThrough();
  const lines = readline.createInterface({ input: output });
  const waiters = new Map();
  lines.on("line", (line) => {
    const response = JSON.parse(line);
    waiters.get(response.id)?.resolve(response);
  });
  const browser = new BrowserCdpClient({ connectionFactory: async () => chromium });
  const server = new BrowserControllerStdioServer({ input, output, browser });
  const running = server.run();
  t.after(async () => {
    blocked = false;
    releaseMouseMove.resolve();
    releaseMousePress.resolve();
    input.end();
    await running;
    await browser.close();
    lines.close();
    output.end();
  });
  const rpc = (id, method, params) => {
    const result = Promise.withResolvers();
    waiters.set(id, result);
    const deadline = setTimeout(() => result.reject(new Error(`${method} response timed out`)), 500);
    input.write(`${JSON.stringify({ id, method, params })}\n`);
    return result.promise.finally(() => { clearTimeout(deadline); waiters.delete(id); });
  };
  const params = { target_id: "page", document_id: "document", node_ref: "backend:1", action: { kind: "click" }, timeout_ms: 2000 };
  const action = rpc(1, "browser.action", params);
  void action.catch(() => {});
  await observed.promise;
  const cancel = await rpc(2, "browser.cancel", { request_id: 1 });
  assert.deepEqual(cancel, { id: 2, ok: true, result: { accepted: true } });
  const cancelled = await action;
  assert.equal(cancelled.ok, false);
  assert.equal(cancelled.error.code, "browser_action_cancelled");
  blocked = false;
  assert.equal((await rpc(3, "browser.action", params)).ok, true);
  assert.deepEqual(await rpc(4, "browser.cancel", { request_id: 1 }), {
    id: 4, ok: true, result: { accepted: false },
  });
  const previousPresses = presses;
  holdMouseMove = true;
  const moving = rpc(5, "browser.action", params);
  void moving.catch(() => {});
  await mouseMoved.promise;
  assert.equal((await rpc(9, "browser.cancel", { request_id: 1 })).result.accepted, false,
    "a late cancellation must not target a different active action");
  assert.equal((await rpc(6, "browser.cancel", { request_id: 5 })).result.accepted, true);
  releaseMouseMove.resolve();
  const stopped = await moving;
  assert.equal(stopped.error?.code, "browser_action_cancelled");
  assert.equal(presses, previousPresses, "cancellation during pointer movement must prevent the click");
  holdMouseMove = false;
  holdMousePress = true;
  const pressing = rpc(7, "browser.action", params);
  void pressing.catch(() => {});
  await mousePressed.promise;
  assert.equal((await rpc(8, "browser.cancel", { request_id: 7 })).result.accepted, true);
  releaseMousePress.resolve();
  assert.equal((await pressing).error?.code, "browser_action_cancelled");
  assert.equal(releases, presses, "a partially pressed gesture must release its button before cancellation finishes");
});
