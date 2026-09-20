import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserOneShotCompatibilityAdapter,
  BrowserOneShotCompatibilityError,
  ONESHOT_COMPAT_ERROR_CODES,
  parseOneShotBrowserCommand,
} from "./browser-controller-oneshot-compat.mjs";
import { BrowserRuntimeMcpAdapter } from "./browser-controller-runtime-mcp.mjs";

function recorder(results = {}) {
  const calls = [];
  return {
    calls,
    adapter: {
      async invoke(toolName, args) {
        calls.push({ toolName, args });
        return { result: results[toolName] ?? { ok: true } };
      },
    },
  };
}

function semanticPort(overrides = {}) {
  return {
    status: async () => ({ ok: true }),
    find: async () => ({ matches: [] }),
    fill: async () => ({ ok: true }),
    click: async () => ({ ok: true }),
    submit: async () => ({ ok: true }),
    dialog: async () => ({ ok: true }),
    text: async () => ({ text: "page text" }),
    waitForText: async () => ({ ok: true }),
    waitForSelector: async () => ({ ok: true }),
    waitForIdle: async () => ({ ok: true }),
    ...overrides,
  };
}

test("maps every supported one-shot command to the canonical runtime tool", async () => {
  const fake = recorder({
    "chariox.slice_browser_text": { text: "body" },
  });
  const adapter = new BrowserOneShotCompatibilityAdapter(fake.adapter);
  const cases = [
    [["status"], "chariox.slice_browser_status", {}],
    [["find", "Email", "field"], "chariox.slice_browser_find", { query: "Email", kind: "field" }],
    [["fill", "#email", "hello", "world"], "chariox.slice_browser_fill", { selector: "#email", text: "hello world" }],
    [["click-selector", "button[type=submit]"], "chariox.slice_browser_click", { selector: "button[type=submit]" }],
    [["submit"], "chariox.slice_browser_submit", {}],
    [["dialog", "accept", "yes"], "chariox.slice_browser_dialog", { action: "accept", prompt_text: "yes" }],
    [["text"], "chariox.slice_browser_text", {}],
    [["wait-text", "Ready", "250"], "chariox.slice_browser_wait_for_text", { text: "Ready", timeout_ms: 250 }],
    [["wait-selector", "main"], "chariox.slice_browser_wait_for_selector", { selector: "main", timeout_ms: 10_000 }],
    [["wait-idle"], "chariox.slice_browser_wait_for_idle", { timeout_ms: 10_000 }],
  ];
  for (const [argv] of cases) await adapter.invoke(argv);
  assert.deepEqual(
    fake.calls,
    cases.map(([, toolName, args]) => ({ toolName, args })),
  );
});

test("preserves legacy opaque targets while keeping CSS selectors distinct", () => {
  assert.deepEqual(parseOneShotBrowserCommand(["click-selector", "field:12"]), {
    tool_name: "chariox.slice_browser_click",
    arguments: { field_id: "field:12" },
    output: "json",
  });
  assert.deepEqual(parseOneShotBrowserCommand(["submit", "body > button:nth-of-type(1)"]), {
    tool_name: "chariox.slice_browser_submit",
    arguments: { selector: "body > button:nth-of-type(1)" },
    output: "json",
  });
  assert.deepEqual(parseOneShotBrowserCommand(["click-selector", "button:first-of-type"]), {
    tool_name: "chariox.slice_browser_click",
    arguments: { selector: "button:first-of-type" },
    output: "json",
  });
});

test("normalizes the wrapper's empty submit argument to no target", () => {
  assert.deepEqual(parseOneShotBrowserCommand(["submit", ""]), {
    tool_name: "chariox.slice_browser_submit",
    arguments: {},
    output: "json",
  });
});

test("reads fill-stdin without placing text in output", async () => {
  const secret = "correct horse battery staple";
  const fake = recorder();
  const adapter = new BrowserOneShotCompatibilityAdapter(fake.adapter);
  const result = await adapter.invoke(["fill-stdin", "element:7"], { stdin: secret });
  assert.deepEqual(fake.calls, [{
    toolName: "chariox.slice_browser_fill",
    args: { field_id: "element:7", text: secret },
  }]);
  assert.equal(result.exit_code, 0);
  assert.equal(result.stdout, '{"ok":true,"field_id":"element:7"}');
  assert.equal(result.stdout.includes(secret), false);
});

test("projects fill output even when the runtime port echoes its arguments", async () => {
  const secret = "vault-backed-password";
  const runtime = new BrowserRuntimeMcpAdapter(semanticPort({
    fill: async (args) => ({ ok: true, args }),
  }));
  const adapter = new BrowserOneShotCompatibilityAdapter(runtime);

  const result = await adapter.invoke(["fill-stdin", "field:42"], { stdin: secret });

  assert.deepEqual(result, {
    exit_code: 0,
    stdout: '{"ok":true,"field_id":"field:42"}',
  });
  assert.equal(result.stdout.includes(secret), false);
});

test("preserves plain-text output and wait failure exit status", async () => {
  const fake = recorder({
    "chariox.slice_browser_text": { text: "page text" },
    "chariox.slice_browser_wait_for_text": { ok: false, reason: "timeout" },
  });
  const adapter = new BrowserOneShotCompatibilityAdapter(fake.adapter);
  assert.deepEqual(await adapter.invoke(["text"]), { exit_code: 0, stdout: "page text" });
  assert.deepEqual(await adapter.invoke(["wait-text", "missing"]), {
    exit_code: 1,
    stdout: '{"ok":false,"reason":"timeout"}',
  });
});

test("integrates with the runtime MCP adapter and keeps canonical output", async () => {
  const runtime = new BrowserRuntimeMcpAdapter(semanticPort());
  const adapter = new BrowserOneShotCompatibilityAdapter(runtime);
  assert.deepEqual(await adapter.invoke(["find", "Continue"]), {
    exit_code: 0,
    stdout: '{"matches":[]}',
  });
  assert.deepEqual(await adapter.invoke(["text"]), {
    exit_code: 0,
    stdout: "page text",
  });
});

test("rejects unsupported or malformed one-shot calls before forwarding", async () => {
  const fake = recorder();
  const adapter = new BrowserOneShotCompatibilityAdapter(fake.adapter);
  for (const argv of [
    [],
    ["navigate", "https://example.com"],
    ["status", "extra"],
    ["find"],
    ["fill", "#email"],
    ["click-selector"],
    ["submit", "one", "two"],
    ["wait-idle", "later"],
  ]) {
    await assert.rejects(
      adapter.invoke(argv),
      (error) => error instanceof BrowserOneShotCompatibilityError,
    );
  }
  assert.deepEqual(fake.calls, []);
});

test("bounds input without reflecting supplied content in errors", () => {
  const secret = "s".repeat(65 * 1024);
  assert.throws(
    () => parseOneShotBrowserCommand(["fill", "#password", secret]),
    (error) =>
      error.code === ONESHOT_COMPAT_ERROR_CODES.INPUT_TOO_LARGE &&
      error.message === ONESHOT_COMPAT_ERROR_CODES.INPUT_TOO_LARGE &&
      !error.message.includes(secret),
  );
});

test("requires text results to contain bounded public text", async () => {
  const fake = recorder({ "chariox.slice_browser_text": { ok: true } });
  const adapter = new BrowserOneShotCompatibilityAdapter(fake.adapter);
  await assert.rejects(
    adapter.invoke(["text"]),
    (error) => error.code === ONESHOT_COMPAT_ERROR_CODES.OUTPUT_INVALID,
  );
});
