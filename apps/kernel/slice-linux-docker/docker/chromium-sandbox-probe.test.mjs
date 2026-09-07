import assert from "node:assert/strict";
import test from "node:test";
import { boundedResponseJson, validateSandboxReport } from "./chromium-sandbox-probe.mjs";

const text = "PID namespaces Yes\nNetwork namespaces Yes\nSeccomp-BPF sandbox Yes\n";
const browser = { uid: 1000, uids: [1000, 1000, 1000, 1000], pidNamespace: "pid:[100]", netNamespace: "net:[100]", seccompFilters: 1 };
const renderer = { ...browser, pidNamespace: "pid:[200]", netNamespace: "net:[200]",
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
