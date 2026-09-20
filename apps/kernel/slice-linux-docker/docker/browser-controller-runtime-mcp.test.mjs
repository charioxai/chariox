import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserRuntimeMcpAdapter,
  BrowserRuntimeMcpError,
  RUNTIME_MCP_ERROR_CODES,
  canonicalRuntimeMcpToolName,
} from "./browser-controller-runtime-mcp.mjs";

function port(overrides = {}) {
  const calls = [];
  const methods = {
    status: async (args) => ({ ok: true, args }),
    find: async (args) => ({ matches: [], args }),
    fill: async (args) => ({ ok: true, args }),
    click: async (args) => ({ ok: true, args }),
    submit: async (args) => ({ ok: true, args }),
    dialog: async (args) => ({ ok: true, args }),
    text: async (args) => ({ text: "page", args }),
    waitForText: async (args) => ({ ok: true, args }),
    waitForSelector: async (args) => ({ ok: true, args }),
    waitForIdle: async (args) => ({ ok: true, args }),
    ...overrides,
  };
  for (const name of Object.keys(methods)) {
    const implementation = methods[name];
    methods[name] = async (args) => {
      calls.push({ name, args });
      return implementation(args);
    };
  }
  return { calls, methods };
}

function assertCode(code, action) {
  assert.throws(action, (error) => error instanceof BrowserRuntimeMcpError && error.code === code);
}

test("canonicalizes every existing public browser tool spelling", () => {
  for (const suffix of [
    "status", "find", "fill", "click", "submit", "dialog", "text",
    "wait_for_text", "wait_for_selector", "wait_for_idle",
  ]) {
    const expected = `chariox.slice_browser_${suffix}`;
    for (const name of [
      expected,
      `slice_browser_${suffix}`,
      `chariox_slice_browser_${suffix}`,
      `mcp__chariox__slice_browser_${suffix}`,
      `mcp__chariox__chariox_slice_browser_${suffix}`,
    ]) {
      assert.equal(canonicalRuntimeMcpToolName(name), expected);
    }
  }
});

test("rejects unknown names without forwarding", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  await assert.rejects(
    adapter.invoke("chariox.slice_browser_eval", {}),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.INVALID_TOOL,
  );
  assert.deepEqual(fake.calls, []);
});

test("forwards canonicalized calls to the semantic controller port", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  const result = await adapter.invoke("mcp__chariox__slice_browser_find", {
    query: "Email",
    kind: "field",
  });
  assert.deepEqual(fake.calls, [{ name: "find", args: { query: "Email", kind: "field" } }]);
  assert.equal(result.tool_name, "chariox.slice_browser_find");
  assert.deepEqual(result.result.matches, []);
});

test("normalizes the optional find kind", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  await adapter.invoke("slice_browser_find", { query: "Continue" });
  assert.deepEqual(fake.calls[0], { name: "find", args: { query: "Continue", kind: "any" } });
});

test("requires exactly one fill or click target", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  for (const args of [
    { text: "hello" },
    { selector: "#email", field_id: "element:1", text: "hello" },
  ]) {
    await assert.rejects(adapter.invoke("slice_browser_fill", args), (error) =>
      error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  await adapter.invoke("slice_browser_fill", { field_id: "element:1", text: "" });
  await adapter.invoke("slice_browser_click", { selector: "button[type=submit]" });
  await assert.rejects(
    adapter.invoke("slice_browser_fill", { field_id: "element:1", text: undefined }),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT,
  );
  assert.equal(fake.calls.length, 2);
});

test("rejects empty and blank targets before forwarding", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  for (const [tool, args] of [
    ["slice_browser_fill", { selector: "", text: "value" }],
    ["slice_browser_fill", { field_id: " \t ", text: "value" }],
    ["slice_browser_click", { selector: "\n" }],
    ["slice_browser_submit", { field_id: " " }],
  ]) {
    await assert.rejects(
      adapter.invoke(tool, args),
      (error) => error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT,
    );
  }
  assert.deepEqual(fake.calls, []);
});

test("keeps submit target optional but unambiguous", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  await adapter.invoke("slice_browser_submit", {});
  await adapter.invoke("slice_browser_submit", { field_id: "element:2" });
  await assert.rejects(
    adapter.invoke("slice_browser_submit", { selector: "form", field_id: "element:2" }),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT,
  );
  assert.deepEqual(fake.calls.map(({ args }) => args), [{}, { field_id: "element:2" }]);
});

test("validates dialog action and prompt semantics", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  await adapter.invoke("slice_browser_dialog", { action: "accept", prompt_text: "yes" });
  await assert.rejects(
    adapter.invoke("slice_browser_dialog", { action: "dismiss", prompt_text: "no" }),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT,
  );
  await assert.rejects(
    adapter.invoke("slice_browser_dialog", { action: "later" }),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT,
  );
});

test("enforces current timeout bounds and exact schemas", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  for (const args of [
    { text: "Ready", timeout_ms: 99 },
    { text: "Ready", timeout_ms: 60_001 },
    { text: "Ready", extra: true },
  ]) {
    await assert.rejects(adapter.invoke("slice_browser_wait_for_text", args), (error) =>
      error.code === RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT);
  }
  await adapter.invoke("slice_browser_wait_for_text", { text: "Ready", timeout_ms: 100 });
  await adapter.invoke("slice_browser_wait_for_selector", { selector: "main", timeout_ms: 60_000 });
  await adapter.invoke("slice_browser_wait_for_idle", {});
  assert.equal(fake.calls.length, 3);
});

test("rejects oversized requests before forwarding", async () => {
  const fake = port();
  const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
  await assert.rejects(
    adapter.invoke("slice_browser_fill", { selector: "#value", text: "x".repeat(65 * 1024) }),
    (error) => [
      RUNTIME_MCP_ERROR_CODES.INVALID_ARGUMENT,
      RUNTIME_MCP_ERROR_CODES.REQUEST_TOO_LARGE,
    ].includes(error.code),
  );
  assert.deepEqual(fake.calls, []);
});

test("rejects every controller-private identifier spelling in results", async () => {
  for (const key of [
    "backend_node_id",
    "backendNodeId",
    "browser_context_id",
    "browserContextId",
    "object_id",
    "objectId",
    "session_id",
    "sessionId",
    "target_id",
    "targetId",
    "websocket_url",
    "websocketUrl",
    "webSocketDebuggerUrl",
  ]) {
    const fake = port({ status: async () => ({ tabs: [{ [key]: "private" }] }) });
    const adapter = new BrowserRuntimeMcpAdapter(fake.methods);
    await assert.rejects(
      adapter.invoke("slice_browser_status", {}),
      (error) => error.code === RUNTIME_MCP_ERROR_CODES.RESULT_INVALID,
      key,
    );
  }
});

test("rejects cyclic and oversized results", async () => {
  const cyclic = {};
  cyclic.self = cyclic;
  const cyclicPort = port({ status: async () => cyclic });
  await assert.rejects(
    new BrowserRuntimeMcpAdapter(cyclicPort.methods).invoke("slice_browser_status", {}),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.RESULT_INVALID,
  );

  const largePort = port({ text: async () => ({ text: "x".repeat(1024 * 1024) }) });
  await assert.rejects(
    new BrowserRuntimeMcpAdapter(largePort.methods).invoke("slice_browser_text", {}),
    (error) => error.code === RUNTIME_MCP_ERROR_CODES.RESULT_TOO_LARGE,
  );
});

test("requires a complete semantic controller port", () => {
  assert.throws(() => new BrowserRuntimeMcpAdapter({}), /port\.status/);
  assertCode(RUNTIME_MCP_ERROR_CODES.INVALID_TOOL, () => canonicalRuntimeMcpToolName("unknown"));
});
