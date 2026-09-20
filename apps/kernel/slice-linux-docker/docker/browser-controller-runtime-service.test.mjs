import { strict as assert } from "node:assert";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "node:net";
import { test } from "node:test";

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
  const root = await mkdtemp(join(tmpdir(), "chariox-browser-runtime-service-"));
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
      /ENOENT|ECONNREFUSED/,
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
    getTabRegistrySnapshot: () => ({
      tabs: [{ tab_id: "tab-1", target_generation: 3 }],
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
    getTabRegistrySnapshot: () => ({
      tabs: [{ tab_id: target.tab_id, target_generation: target.target_generation }],
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
