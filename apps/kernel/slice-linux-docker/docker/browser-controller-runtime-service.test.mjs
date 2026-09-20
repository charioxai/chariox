import { strict as assert } from "node:assert";
import { AsyncLocalStorage } from "node:async_hooks";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "node:net";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

import {
  BrowserRuntimeMcpAdapter,
} from "./browser-controller-runtime-mcp.mjs";
import {
  BrowserRuntimeSemanticPort,
  BrowserRuntimeMcpService,
  RUNTIME_ERROR_CODES,
} from "./browser-controller-runtime-service.mjs";

function request(socketPath, value, timeoutMs = 1_000) {
  return new Promise((resolve, reject) => {
    const socket = connect(socketPath);
    let data = "";
    let settled = false;
    const finish = (callback, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      callback(result);
    };
    const timer = setTimeout(
      () => finish(reject, new Error("test request timed out")),
      timeoutMs,
    );
    socket.setEncoding("utf8");
    socket.on("connect", () => {
      socket.write(typeof value === "string" ? value : JSON.stringify(value) + "\n");
    });
    socket.on("data", (chunk) => {
      data += chunk;
      const newline = data.indexOf("\n");
      if (newline < 0) return;
      try {
        finish(resolve, JSON.parse(data.slice(0, newline)));
      } catch (error) {
        finish(reject, error);
      }
    });
    socket.on("error", (error) => finish(reject, error));
  });
}

function authenticatedRequest(socketPath, authToken, requestId, toolName, argumentsValue) {
  return request(socketPath, {
    type: "tool_call",
    request_id: requestId,
    auth_token: authToken,
    tool_name: toolName,
    arguments: argumentsValue,
  });
}

test("runtime service owns one semantic endpoint for status and mutation", async () => {
  const root = await mkdtemp(join(tmpdir(), "cbx-"));
  const socketPath = join(root, "browser-runtime-mcp.sock");
  const authToken = "local-test-token";
  const calls = [];
  let controllerStarted = false;
  let hangStatus = false;
  const port = {
    startController: async () => {
      controllerStarted = true;
    },
    status: async () => {
      calls.push("status");
      if (hangStatus) await new Promise(() => {});
      return {
        url: "https://example.test/login",
        fields: [{ selector: "#email", field_id: "field:1" }],
      };
    },
    find: async () => {
      calls.push("find");
      return { matches: [] };
    },
    fill: async (args) => {
      calls.push("fill");
      return { ok: true, selector: args.selector ?? args.field_id };
    },
    click: async () => {
      calls.push("click");
      return { ok: true };
    },
    submit: async () => {
      calls.push("submit");
      return { ok: true };
    },
    dialog: async () => {
      calls.push("dialog");
      return { ok: true };
    },
    text: async () => {
      calls.push("text");
      return "";
    },
    waitForText: async () => {
      calls.push("waitForText");
      return { ok: true };
    },
    waitForSelector: async () => {
      calls.push("waitForSelector");
      return { ok: true };
    },
    waitForIdle: async () => {
      calls.push("waitForIdle");
      return { ok: true };
    },
  };
  const service = new BrowserRuntimeMcpService({
    socketPath,
    authToken,
    adapter: new BrowserRuntimeMcpAdapter(port),
    requestTimeoutMs: 100,
  });

  try {
    await service.start();
    assert.equal(controllerStarted, true);
    const health = await request(socketPath, {
      type: "health",
      request_id: "health-1",
      auth_token: authToken,
    });
    assert.equal(health.ok, true);
    assert.equal(health.state, "listening");

    const status = await authenticatedRequest(
      socketPath,
      authToken,
      "status-1",
      "slice_browser_status",
      {},
    );
    assert.equal(status.ok, true);
    assert.equal(status.tool_name, "chariox.slice_browser_status");
    assert.equal(status.result.url, "https://example.test/login");

    const fill = await authenticatedRequest(
      socketPath,
      authToken,
      "fill-1",
      "mcp__chariox__slice_browser_fill",
      { field_id: "field:1", text: "alice@example.test" },
    );
    assert.equal(fill.ok, true);
    assert.deepEqual(fill.result, { ok: true, selector: "field:1" });
    assert.deepEqual(calls, ["status", "fill"]);

    const source = await readFile(new URL("./browser-controller-runtime-service.mjs", import.meta.url), "utf8");
    assert.doesNotMatch(source, /browser-cdp\.mjs|slice-screen\.sh/);

    const malformed = await request(socketPath, "{malformed\n");
    assert.equal(malformed.ok, false);
    assert.equal(malformed.error.code, RUNTIME_ERROR_CODES.MALFORMED);

    hangStatus = true;
    const timedOut = await authenticatedRequest(
      socketPath,
      authToken,
      "status-timeout",
      "chariox.slice_browser_status",
      {},
    );
    assert.equal(timedOut.ok, false);
    assert.equal(timedOut.error.code, RUNTIME_ERROR_CODES.TIMEOUT);
  } finally {
    await service.stop();
    await assert.rejects(
      request(socketPath, {
        type: "health",
        request_id: "unavailable-1",
        auth_token: authToken,
      }),
      /ENOENT|ECONNREFUSED|EINVAL/,
    );
    await rm(root, { recursive: true, force: true });
  }
});

test("semantic mutation dispatch supplies the controller attribution seam", async () => {
  const calls = [];
  const targetCalls = [];
  const target = {
    tab_id: "tab-1",
    target_id: "target-a",
    target_type: "page",
    generation: 7,
    target_generation: 3,
    websocket_url: "ws://target-a",
  };
  const targetConnection = {
    send: async (method) => {
      targetCalls.push(method);
      if (method === "DOM.getDocument") return { root: { nodeId: 1 } };
      if (method === "DOM.querySelector") return { nodeId: 2 };
      if (method === "DOM.describeNode") return { node: { backendNodeId: 42 } };
      throw new Error(`unexpected CDP method ${method}`);
    },
  };
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: { send: async () => { throw new Error("wrong browser-level target"); } },
    _enqueue: async (_generation, operation) => operation(),
    refreshTabs: async () => {},
    getTabRegistrySnapshot: () => ({
      tabs: [{ tab_id: "tab-1", target_type: "page", target_generation: 3 }],
    }),
    tabRegistry: {
      resolveTarget: (tabId, options) => {
        assert.equal(tabId, target.tab_id);
        assert.equal(options.generation, target.generation);
        assert.equal(options.target_generation, target.target_generation);
        return { ...target };
      },
    },
    _ensureTargetConnection: async (resolvedTarget) => {
      assert.equal(resolvedTarget.target_id, target.target_id);
      return targetConnection;
    },
    captureTabSnapshot: () => ({
      accessibility_nodes: [{ element_ref: "element-1" }],
      dom_nodes: [],
    }),
    resolveElementReference: () => ({ backend_node_id: 42 }),
    performElementAction: async (ownerId, generation, request, mutation) => {
      calls.push({ ownerId, generation, request, mutation });
    },
    cancelMutation: () => ({ accepted: false, state: "unknown" }),
  };
  const requestContext = { getStore: () => ({ requestId: "request-1" }) };
  const semantic = new BrowserRuntimeSemanticPort(controller, "kernel:slice-1", requestContext);

  const result = await semantic.fill({ selector: "#email", text: "alice@example.test" });

  assert.deepEqual(result, { ok: true, selector: "#email" });
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].mutation, {
    action_id: calls[0].mutation.action_id,
    actor_id: "kernel:slice-1",
  });
  assert.match(calls[0].mutation.action_id, /^runtime-\d+-\d+$/);
  assert.deepEqual(targetCalls, ["DOM.getDocument", "DOM.querySelector", "DOM.describeNode"]);
  assert.deepEqual(calls[0].request, {
    tab_id: "tab-1",
    target_generation: 3,
    element_ref: "element-1",
    action: { kind: "fill", text: "alice@example.test", append: false },
  });
});

test("rejects a target rotation between selector discovery and mutation", async () => {
  const target = {
    tab_id: "tab-1",
    target_id: "target-a",
    target_type: "page",
    generation: 7,
    target_generation: 3,
    websocket_url: "ws://target-a",
  };
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: {},
    _enqueue: async (_generation, operation) => operation(),
    refreshTabs: async () => {},
    getTabRegistrySnapshot: () => ({
      tabs: [{
        tab_id: target.tab_id,
        target_type: "page",
        target_generation: target.target_generation,
      }],
    }),
    tabRegistry: {
      resolveTarget: (tabId, options) => {
        if (options.target_generation !== target.target_generation) {
          const error = new Error("stale target generation");
          error.code = "STALE_TARGET_GENERATION";
          throw error;
        }
        return { ...target, tab_id: tabId };
      },
    },
    _ensureTargetConnection: async () => ({
      send: async (method) => {
        if (method === "DOM.getDocument") return { root: { nodeId: 1 } };
        if (method === "DOM.querySelector") return { nodeId: 2 };
        if (method === "DOM.describeNode") return { node: { backendNodeId: 42 } };
        throw new Error(`unexpected CDP method ${method}`);
      },
    }),
    captureTabSnapshot: () => ({
      accessibility_nodes: [{ element_ref: "element-1" }],
      dom_nodes: [],
    }),
    resolveElementReference: () => {
      target.target_generation = 4;
      return { backend_node_id: 42 };
    },
    performElementAction: async (_ownerId, _generation, request) => {
      if (request.target_generation !== target.target_generation) {
        const error = new Error("stale target generation");
        error.code = "STALE_TARGET_GENERATION";
        throw error;
      }
      throw new Error("mutation should not run for this regression");
    },
    cancelMutation: () => ({ accepted: false, state: "unknown" }),
  };
  const semantic = new BrowserRuntimeSemanticPort(controller, "kernel:slice-1", {
    getStore: () => ({ requestId: "request-rotation" }),
  });

  await assert.rejects(
    semantic.fill({ selector: "#email", text: "alice@example.test" }),
    (error) => error.code === "STALE_TARGET_GENERATION",
  );
});

test("refreshes targets and selects the focused live page deterministically", async () => {
  let tabs = [
    { tab_id: "tab-a", target_id: "target-a", target_type: "page", target_generation: 1 },
    { tab_id: "tab-b", target_id: "target-b", target_type: "page", target_generation: 1 },
  ];
  let focusedTab = "tab-b";
  let refreshes = 0;
  const connections = new Map();
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: {},
    _enqueue: async (_generation, operation) => operation(),
    refreshTabs: async () => { refreshes += 1; },
    getTabRegistrySnapshot: () => ({ tabs }),
    tabRegistry: {
      resolveTarget: (tabId, options) => ({
        ...tabs.find((tab) => tab.tab_id === tabId),
        generation: options.generation,
        websocket_url: `ws://${tabId}`,
      }),
    },
    _ensureTargetConnection: async (target) => {
      if (!connections.has(target.tab_id)) {
        connections.set(target.tab_id, {
          send: async (method, params) => {
            assert.equal(method, "Runtime.evaluate");
            if (params.expression === "Boolean(document.hasFocus() || document.visibilityState === 'visible')") {
              return { result: { value: target.tab_id === focusedTab } };
            }
            return {
              result: {
                value: {
                  url: `https://example.test/${target.tab_id}`,
                  fields: [],
                  buttons: [],
                  links: [],
                },
              },
            };
          },
        });
      }
      return connections.get(target.tab_id);
    },
  };
  const semantic = new BrowserRuntimeSemanticPort(controller, "kernel:slice-1");

  assert.equal((await semantic.status()).url, "https://example.test/tab-b");
  tabs = [
    { tab_id: "tab-c", target_id: "target-c", target_type: "page", target_generation: 1 },
    { tab_id: "tab-a", target_id: "target-a", target_type: "page", target_generation: 1 },
  ];
  focusedTab = "tab-c";
  assert.equal((await semantic.status()).url, "https://example.test/tab-c");
  tabs = [tabs[1], tabs[0]];
  focusedTab = null;
  assert.equal((await semantic.status()).url, "https://example.test/tab-c");
  tabs = [tabs[0]];
  assert.equal((await semantic.status()).url, "https://example.test/tab-a");
  assert.equal(refreshes, 4);
});

test("find filters the complete visible candidate set before bounding matches", async () => {
  const elements = Array.from({ length: 40 }, (_, index) => ({
    id: `field-${index + 1}`,
    name: "",
    tagName: "INPUT",
    nodeType: 1,
    parentElement: null,
    disabled: false,
    readOnly: false,
    isContentEditable: false,
    innerText: "",
    value: index === 32 ? "needle-33" : `value-${index + 1}`,
    getAttribute(name) {
      if (name === "type") return "text";
      return "";
    },
    getBoundingClientRect: () => ({ width: 10, height: 10 }),
    closest: () => null,
  }));
  const document = {
    body: { children: [] },
    activeElement: null,
    querySelector: () => null,
    querySelectorAll(selector) {
      return selector.startsWith("input, textarea") ? elements : [];
    },
  };
  document.activeElement = document.body;
  const cdp = {
    send: async (method, params) => {
      assert.equal(method, "Runtime.evaluate");
      return {
        result: {
          value: runInNewContext(params.expression, {
            CSS: { escape: (value) => String(value) },
            Node: { ELEMENT_NODE: 1 },
            document,
            location: { href: "https://example.test", host: "example.test" },
            window: {
              getComputedStyle: () => ({ visibility: "visible", display: "block" }),
            },
          }),
        },
      };
    },
  };
  const tab = {
    tab_id: "tab-1",
    target_id: "target-1",
    target_type: "page",
    target_generation: 1,
  };
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: {},
    _enqueue: async (_generation, operation) => operation(),
    refreshTabs: async () => {},
    getTabRegistrySnapshot: () => ({ tabs: [tab] }),
    tabRegistry: {
      resolveTarget: () => ({ ...tab, generation: 7, websocket_url: "ws://target-1" }),
    },
    _ensureTargetConnection: async () => cdp,
  };
  const result = await new BrowserRuntimeSemanticPort(controller, "kernel:slice-1")
    .find({ query: "needle-33", kind: "field" });

  assert.equal(result.matches.length, 1);
  assert.equal(result.matches[0].selector, "#field-33");
  assert.equal(result.truncated, false);
});

test("submit keeps the coordinated mutation path for a form without a submit control", async () => {
  const target = {
    tab_id: "tab-1",
    target_id: "target-a",
    target_type: "page",
    generation: 7,
    target_generation: 3,
    websocket_url: "ws://target-a",
  };
  const mutations = [];
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: {},
    _enqueue: async (_generation, operation) => operation(),
    refreshTabs: async () => {},
    getTabRegistrySnapshot: () => ({ tabs: [target] }),
    tabRegistry: { resolveTarget: () => ({ ...target }) },
    _ensureTargetConnection: async () => ({
      send: async (method, params) => {
        if (method === "Runtime.evaluate") return { result: { value: "#email" } };
        if (method === "DOM.getDocument") return { root: { nodeId: 1 } };
        if (method === "DOM.querySelector") return { nodeId: 2 };
        if (method === "DOM.describeNode") return { node: { backendNodeId: 42 } };
        throw new Error(`unexpected CDP method ${method}: ${JSON.stringify(params)}`);
      },
    }),
    captureTabSnapshot: () => ({
      accessibility_nodes: [{ element_ref: "element-1" }],
      dom_nodes: [],
    }),
    resolveElementReference: () => ({ backend_node_id: 42 }),
    performElementAction: async (_ownerId, _generation, request) => { mutations.push(request); },
    cancelMutation: () => ({ accepted: false, state: "unknown" }),
  };

  await new BrowserRuntimeSemanticPort(controller, "kernel:slice-1").submit({ selector: "form" });

  assert.equal(mutations.length, 1);
  assert.deepEqual(mutations[0].action, { kind: "submit" });
  assert.equal(mutations[0].element_ref, "element-1");
});

test("an aborted queued request cannot enter the mutation boundary", async () => {
  let queue = Promise.resolve();
  let waitStartedResolve;
  const waitStarted = new Promise((resolve) => { waitStartedResolve = resolve; });
  const target = {
    tab_id: "tab-1",
    target_id: "target-a",
    target_type: "page",
    generation: 7,
    target_generation: 3,
    websocket_url: "ws://target-a",
  };
  let mutationCount = 0;
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: {},
    _enqueue(_generation, operation) {
      const current = queue.then(operation);
      queue = current.catch(() => {});
      return current;
    },
    refreshTabs() { return this._enqueue(7, async () => {}); },
    getTabRegistrySnapshot: () => ({ tabs: [target] }),
    tabRegistry: { resolveTarget: () => ({ ...target }) },
    _ensureTargetConnection: async () => ({
      send: async (method) => {
        if (method === "Runtime.evaluate") {
          waitStartedResolve();
          return { result: { value: { ok: false } } };
        }
        if (method === "DOM.getDocument") return { root: { nodeId: 1 } };
        if (method === "DOM.querySelector") return { nodeId: 2 };
        if (method === "DOM.describeNode") return { node: { backendNodeId: 42 } };
        throw new Error(`unexpected CDP method ${method}`);
      },
    }),
    captureTabSnapshot: () => ({
      accessibility_nodes: [{ element_ref: "element-1" }],
      dom_nodes: [],
    }),
    resolveElementReference: () => ({ backend_node_id: 42 }),
    performElementAction: async () => { mutationCount += 1; },
    cancelMutation: () => ({ accepted: false, state: "unknown" }),
  };
  const requestContext = new AsyncLocalStorage();
  const semantic = new BrowserRuntimeSemanticPort(controller, "kernel:slice-1", requestContext);
  const authToken = "local-test-token";
  const service = new BrowserRuntimeMcpService({
    socketPath: "/unused",
    authToken,
    adapter: new BrowserRuntimeMcpAdapter({
      ...Object.fromEntries([
        "status", "find", "fill", "click", "submit", "dialog", "text",
        "waitForText", "waitForSelector", "waitForIdle",
      ].map((name) => [name, (...args) => semantic[name](...args)])),
      cancelMutationForRequest: (requestId) => semantic.cancelMutationForRequest(requestId),
    }),
    requestContext,
    requestTimeoutMs: 1_000,
  });
  const waitAbort = new AbortController();
  const fillAbort = new AbortController();
  const encode = (requestId, toolName, args) => Buffer.from(JSON.stringify({
    type: "tool_call",
    request_id: requestId,
    auth_token: authToken,
    tool_name: toolName,
    arguments: args,
  }));
  const waiting = service.handleLine(
    encode("wait-1", "slice_browser_wait_for_text", { text: "ready", timeout_ms: 100 }),
    waitAbort,
  );
  await waitStarted;
  const filling = service.handleLine(
    encode("fill-1", "slice_browser_fill", { selector: "#email", text: "late" }),
    fillAbort,
  );
  fillAbort.abort();

  const [waitResult, fillResult] = await Promise.all([waiting, filling]);
  assert.equal(waitResult.ok, true);
  assert.equal(fillResult.ok, false);
  assert.equal(fillResult.error.code, "REQUEST_CANCELLED");
  assert.equal(mutationCount, 0);
});

test("closing a runtime connection aborts every queued request", async () => {
  const root = await mkdtemp(join(tmpdir(), "cbx-"));
  const socketPath = join(root, "browser-runtime-mcp.sock");
  const authToken = "local-test-token";
  let queue = Promise.resolve();
  let waitStartedResolve;
  const waitStarted = new Promise((resolve) => { waitStartedResolve = resolve; });
  const target = {
    tab_id: "tab-1",
    target_id: "target-a",
    target_type: "page",
    generation: 7,
    target_generation: 3,
    websocket_url: "ws://target-a",
  };
  let mutationCount = 0;
  const controller = {
    state: "ready",
    generation: 7,
    cdpCommandTimeoutMs: 100,
    _cdp: {},
    _enqueue(_generation, operation) {
      const current = queue.then(operation);
      queue = current.catch(() => {});
      return current;
    },
    refreshTabs() { return this._enqueue(7, async () => {}); },
    getTabRegistrySnapshot: () => ({ tabs: [target] }),
    tabRegistry: { resolveTarget: () => ({ ...target }) },
    _ensureTargetConnection: async () => ({
      send: async (method) => {
        if (method === "Runtime.evaluate") {
          waitStartedResolve();
          return { result: { value: { ok: false } } };
        }
        if (method === "DOM.getDocument") return { root: { nodeId: 1 } };
        if (method === "DOM.querySelector") return { nodeId: 2 };
        if (method === "DOM.describeNode") return { node: { backendNodeId: 42 } };
        throw new Error(`unexpected CDP method ${method}`);
      },
    }),
    captureTabSnapshot: () => ({
      accessibility_nodes: [{ element_ref: "element-1" }],
      dom_nodes: [],
    }),
    resolveElementReference: () => ({ backend_node_id: 42 }),
    performElementAction: async () => { mutationCount += 1; },
    cancelMutation: () => ({ accepted: false, state: "unknown" }),
  };
  const requestContext = new AsyncLocalStorage();
  const semantic = new BrowserRuntimeSemanticPort(controller, "kernel:slice-1", requestContext);
  const service = new BrowserRuntimeMcpService({
    socketPath,
    authToken,
    adapter: new BrowserRuntimeMcpAdapter({
      ...Object.fromEntries([
        "status", "find", "fill", "click", "submit", "dialog", "text",
        "waitForText", "waitForSelector", "waitForIdle",
      ].map((name) => [name, (...args) => semantic[name](...args)])),
      cancelMutationForRequest: (requestId) => semantic.cancelMutationForRequest(requestId),
    }),
    requestContext,
    requestTimeoutMs: 1_000,
  });
  let socket;
  try {
    await service.start();
    socket = connect(socketPath);
    await new Promise((resolve, reject) => {
      socket.once("connect", resolve);
      socket.once("error", reject);
    });
    const encode = (requestId, toolName, args) => JSON.stringify({
      type: "tool_call",
      request_id: requestId,
      auth_token: authToken,
      tool_name: toolName,
      arguments: args,
    });
    socket.write(`${encode("wait-1", "slice_browser_wait_for_text", { text: "ready", timeout_ms: 100 })}\n${encode("fill-1", "slice_browser_fill", { selector: "#email", text: "late" })}\n`);
    await waitStarted;
    socket.destroy();
    await new Promise((resolve) => setTimeout(resolve, 150));
    assert.equal(mutationCount, 0);
  } finally {
    socket?.destroy();
    await service.stop();
    await rm(root, { recursive: true, force: true });
  }
});
