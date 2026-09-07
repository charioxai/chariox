import assert from "node:assert/strict";
import test from "node:test";
import { boundedResponseJson, formatProbeFailure, namespaceLink, validateSandboxReport, withSandboxText } from "./chromium-sandbox-probe.mjs";

const text = "PID namespaces Yes\nNetwork namespaces Yes\nSeccomp-BPF sandbox Yes\n";
const browser = { pid: 40, namespacePids: [40], uid: 1000, uids: [1000, 1000, 1000, 1000], pidNamespace: "pid:[100]", netNamespace: "net:[100]", seccompFilters: 1 };
const renderer = { ...browser, pid: 50, namespacePids: [50, 1], pidNamespace: "pid:[200]", netNamespace: "net:[200]",
  capabilities: "0000000000000000", noNewPrivileges: 1, seccomp: 2, seccompFilters: 2 };

test("diagnostic labels require independent renderer namespace and seccomp evidence", () => {
  assert.doesNotThrow(() => validateSandboxReport(text, browser, [renderer]));
  for (const change of [
    { pidNamespace: browser.pidNamespace }, { netNamespace: browser.netNamespace },
    { pidNamespace: undefined }, { seccompFilters: 1 }, { seccompFilters: Infinity },
    { noNewPrivileges: 0 }, { seccomp: 0 }, { capabilities: "0000000000000001" },
  ]) assert.throws(() => validateSandboxReport(text, browser, [{ ...renderer, ...change }]));
  assert.throws(() => validateSandboxReport(text, browser, []));
  assert.throws(() => validateSandboxReport(text.replace("PID namespaces Yes", "PID namespaces No"), browser, [renderer]));
});

test("real UID alone cannot hide root effective, saved or filesystem identity", () => {
  for (let index = 0; index < 4; index++) {
    const uids = [...renderer.uids];
    uids[index] = 0;
    assert.throws(() => validateSandboxReport(text, browser, [{ ...renderer, uids }]));
    assert.throws(() => validateSandboxReport(text, { ...browser, uids }, [renderer]));
  }
  assert.throws(() => validateSandboxReport(text, browser, [{ ...renderer, uid: 2000, uids: [2000, 2000, 2000, 2000] }]));
});

test("every renderer must pass, with observed browser baseline", () => {
  assert.throws(() => validateSandboxReport(text, browser, [renderer, { ...renderer, seccomp: 0 }]));
  assert.throws(() => validateSandboxReport(text, { ...browser, seccompFilters: NaN }, [renderer]));
  assert.throws(() => validateSandboxReport(text, { ...browser, pidNamespace: undefined }, [renderer]));
});

test("debugger version response is bounded across streamed chunks and cancelled on overflow", async () => {
  assert.deepEqual(await boundedResponseJson(new Response('{"Browser":"Chromium"}')), { Browser: "Chromium" });
  let cancelled = false;
  let reads = 0;
  const response = new Response(new ReadableStream({
    pull(controller) { reads++; controller.enqueue(new Uint8Array(9)); },
    cancel() { cancelled = true; },
  }));
  await assert.rejects(boundedResponseJson(response, 16), /oversized/);
  assert.equal(cancelled, true);
  assert.ok(reads <= 4);
  await assert.rejects(boundedResponseJson(new Response(new Uint8Array([0xc0, 0x80]))));
});

// A failed hosted probe must identify the assertion without emitting browser
// contents, process argv, profile paths, or arbitrary parser/transport errors.
test("probe failures expose only fixed local diagnostic messages", () => {
  let failure;
  try { validateSandboxReport(text, browser, []); } catch (error) { failure = error; }
  assert.match(formatProbeFailure(failure), /no observable browser renderer processes$/);
  const prefix = "Chromium sandbox/profile verification failed";
  for (const error of [new Error("secret profile path"), { message: "secret page data" }, "secret bytes"])
    assert.equal(formatProbeFailure(error), prefix);
});

test("owned diagnostic target survives process inspection and closes on either outcome", async t => {
  const methods = [];
  class DebuggerSocket extends EventTarget {
    static OPEN = 1;
    readyState = 1;
    constructor() { super(); queueMicrotask(() => this.dispatchEvent(new Event("open"))); }
    send(encoded) {
      const { id, method } = JSON.parse(encoded);
      methods.push(method);
      const result = method === "Target.createTarget" ? { targetId: "owned-diagnostic" }
        : method === "Target.attachToTarget" ? { sessionId: "diagnostic-session" }
        : method === "Runtime.evaluate" ? { result: { value: text } } : {};
      queueMicrotask(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ id, result }) })));
    }
    close() { this.readyState = 3; methods.push("socket.close"); this.dispatchEvent(new Event("close")); }
  }
  t.mock.method(globalThis, "fetch", async () => new Response(JSON.stringify({ webSocketDebuggerUrl: "ws://127.0.0.1:9222/devtools/browser/owned" })));
  const original = globalThis.WebSocket;
  globalThis.WebSocket = DebuggerSocket;
  try {
    for (const fail of [false, true]) {
      methods.length = 0;
      const inspect = async report => {
        assert.equal(report, text);
        assert.equal(methods.includes("Target.closeTarget"), false);
        assert.equal(methods.includes("socket.close"), false);
        await Promise.resolve();
        methods.push("inspect.finished");
        if (fail) throw new Error("inspection rejected");
        return "verified";
      };
      if (fail) await assert.rejects(withSandboxText(inspect), /inspection rejected/);
      else assert.equal(await withSandboxText(inspect), "verified");
      assert.deepEqual(methods.slice(-3), ["inspect.finished", "Target.closeTarget", "socket.close"]);
    }
  } finally { globalThis.WebSocket = original; }
});

test("ptrace-protected namespace links retain independent process sandbox checks", async () => {
  const protectedRenderer = { ...renderer, pidNamespace: null, netNamespace: null,
    pidNamespaceRestricted: true, netNamespaceRestricted: true };
  assert.deepEqual(validateSandboxReport(text, browser, [protectedRenderer]),
    { rendererCount: 1, restrictedPidNamespaceLinks: 1, restrictedNetworkNamespaceLinks: 1 });
  assert.deepEqual(validateSandboxReport(text, browser, [renderer, protectedRenderer]),
    { rendererCount: 2, restrictedPidNamespaceLinks: 1, restrictedNetworkNamespaceLinks: 1 });
  assert.deepEqual(validateSandboxReport(text, browser, [renderer]),
    { rendererCount: 1, restrictedPidNamespaceLinks: 0, restrictedNetworkNamespaceLinks: 0 });
  for (const change of [{ namespacePids: [50] }, { namespacePids: [] }, { namespacePids: [99, 1] },
    { pidNamespaceRestricted: false }, { netNamespaceRestricted: false }, { seccompFilters: 1 }, { noNewPrivileges: 0 }])
    assert.throws(() => validateSandboxReport(text, browser, [{ ...protectedRenderer, ...change }]));
  assert.throws(() => validateSandboxReport(text.replace("Network namespaces Yes", "Network namespaces No"), browser, [protectedRenderer]));
  for (const code of ["EACCES", "EPERM"])
    assert.deepEqual(await namespaceLink("/fixture", async () => { throw Object.assign(new Error("private"), { code }); }),
      { value: null, restricted: true });
  for (const code of ["ENOENT", "EIO"])
    await assert.rejects(namespaceLink("/fixture", async () => { throw Object.assign(new Error("private"), { code }); }), { code });
});
