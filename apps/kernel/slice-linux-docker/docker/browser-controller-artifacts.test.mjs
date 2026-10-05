// MP-08/MP-10/MP-11: first-party byte, provenance and redaction regressions.
import assert from "node:assert/strict";
import test from "node:test";
import { mkdtemp, mkdir, writeFile, readFile, rm, symlink, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { inflateSync } from "node:zlib";
import { BrowserPassiveCapture, artifactBytes, readCompletedDownload, withUploadArtifacts } from "./browser-controller-artifacts.mjs";
import { blackPng } from "./browser-controller-image.mjs";
import { uploadBrowserFiles } from "./browser-controller-files.mjs";

test("MP-08/MP-10/MP-11 completed downloads require current observed identity and exact bytes", async t => {
  const directory = await mkdtemp(path.join(tmpdir(), "chariox-b207-download-test-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const bytes = Buffer.from([0, 255, 1, 2]); await writeFile(path.join(directory, "guid-a"), bytes);
  const download = { state: "completed", targetId: "tab-a", documentId: "doc-a", browserGeneration: 1, filename: "exact.bin" };
  const options = { directory, downloads: new Map([["guid-a", download]]), guid: "guid-a", targetId: "tab-a", documentId: "doc-a", browserGeneration: 1 };
  assert.deepEqual(await readCompletedDownload(options), artifactBytes(bytes, "exact.bin", "application/octet-stream"));
  for (const overrides of [{ targetId: "tab-b" }, { documentId: "doc-b" }, { browserGeneration: 2 }, { guid: "../private" }, { guid: "missing" }]) {
    await assert.rejects(readCompletedDownload({ ...options, ...overrides }));
  }
  for (const state of ["inProgress", "canceled"]) { download.state = state; await assert.rejects(readCompletedDownload(options)); }
  download.state = "completed";
  await rm(path.join(directory, "guid-a")); await symlink("/etc/hostname", path.join(directory, "guid-a"));
  await assert.rejects(readCompletedDownload(options));
});

test("MP-08/MP-10/MP-11 approved uploads preserve names/bytes and reject tampering before delivery", async () => {
  const expected = artifactBytes(Buffer.from("Unicode λ\n"), "réport.txt", "text/plain");
  let staged;
  await withUploadArtifacts([expected], undefined, async (files, roots) => {
    assert.equal(path.basename(files[0]), "réport.txt"); assert.deepEqual(await readFile(files[0]), Buffer.from("Unicode λ\n"));
    assert.ok(files[0].startsWith(roots[0])); staged = roots[0];
  });
  await assert.rejects(readdir(staged), { code: "ENOENT" });
  for (const override of [{ display_name: "../private" }, { size_bytes: 2 }, { sha256: "0".repeat(64) }, { data_base64: "!!!" }]) {
    await assert.rejects(withUploadArtifacts([{ ...expected, ...override }], undefined, () => assert.fail("unapproved bytes delivered")));
  }
  const abort = new AbortController(); abort.abort();
  await assert.rejects(withUploadArtifacts([expected], abort.signal, () => assert.fail("cancelled bytes delivered")), { code: "browser_action_cancelled" });
});

test("MP-08/MP-10/MP-11 actual extra-info headers stay aligned across redirects without private metadata", () => {
  const capture = new BrowserPassiveCapture(); const context = { targetId: "tab-a", documentId: "doc-a" };
  const event = (method, params) => capture.record({ method, sessionId: "session-a", params: { requestId: "req-a", ...params } }, context);
  event("Network.requestWillBeSentExtraInfo", { headers: { Accept: "text/html", Cookie: "secret-cookie", Authorization: "secret-auth" } });
  event("Network.requestWillBeSent", { loaderId: "doc-a", request: { method: "GET", url: "https://user:pass@example.test/one?secret-query#private", postData: "secret-body" } });
  event("Network.requestWillBeSent", { loaderId: "doc-a", request: { method: "GET", url: "https://example.test/two" }, redirectHasExtraInfo: true, redirectResponse: { status: 302, headers: { "Set-Cookie": "secret-response" } } });
  event("Network.requestWillBeSentExtraInfo", { headers: { Accept: "application/json", Cookie: "private-cookie" } });
  event("Network.responseReceived", { hasExtraInfo: true, response: { status: 200, mimeType: "text/html", headers: { "Content-Type": "text/html", "Set-Cookie": "secret" } } });
  const artifact = capture.capture("tab-a", "doc-a");
  const text = Buffer.from(artifact.data_base64, "base64").toString();
  const entries = JSON.parse(text).log.entries;
  assert.equal(entries.length, 2); assert.deepEqual(entries[0].request.headers, [{ name: "accept", value: "text/html" }]);
  assert.deepEqual(entries[1].request.headers, [{ name: "accept", value: "application/json" }]);
  assert.equal(entries[0].response.status, 302); assert.equal(entries[1].response.status, 200);
  assert.equal(entries[0].request.url, "https://example.test/one");
  assert.doesNotMatch(text, /secret|private-cookie|Authorization|Set-Cookie|postData|user:pass/i);
  assert.equal(JSON.parse(Buffer.from(capture.capture("tab-b", "doc-a").data_base64, "base64")).log.entries.length, 0);
  capture.clear(); assert.equal(capture.entries.length, 0);
});

test("MP-08/MP-10/MP-11 a redirect without extra info never steals later actual headers", () => {
  const capture = new BrowserPassiveCapture();
  const event = (method, params) => capture.record({ method, sessionId: "session-a", params: { requestId: "req-a", ...params } }, { targetId: "tab-a", documentId: "doc-a" });
  event("Network.requestWillBeSent", { loaderId: "doc-a", request: { method: "GET", url: "https://example.test/one" } });
  event("Network.requestWillBeSent", { loaderId: "doc-a", redirectHasExtraInfo: false,
    redirectResponse: { status: 302 }, request: { method: "GET", url: "https://example.test/two" } });
  event("Network.requestWillBeSentExtraInfo", { headers: { Accept: "application/json" } });
  assert.deepEqual(capture.entries[0].request.headers, []);
  assert.deepEqual(capture.entries[1].request.headers, []);
  event("Network.responseReceived", { hasExtraInfo: true, response: { status: 200 } });
  assert.deepEqual(capture.entries[0].request.headers, []);
  assert.deepEqual(capture.entries[1].request.headers, [{ name: "accept", value: "application/json" }]);
  assert.equal(capture.entries[1].request.extra_info_observed, true);
});

test("MP-08/MP-10/MP-11 protected frames contain only black pixels and bounded PNG geometry", () => {
  const bytes = blackPng(8, 6); assert.equal(bytes.readUInt32BE(16), 8); assert.equal(bytes.readUInt32BE(20), 6);
  let offset = 8; let compressed;
  while (offset < bytes.length) { const size = bytes.readUInt32BE(offset); if (bytes.toString("ascii", offset + 4, offset + 8) === "IDAT") compressed = bytes.subarray(offset + 8, offset + 8 + size); offset += size + 12; }
  assert.deepEqual(inflateSync(compressed), Buffer.alloc((8 * 3 + 1) * 6));
  assert.throws(() => blackPng(0, 6)); assert.throws(() => blackPng(10000, 6));
});

for (const scenario of ["matching", "wrong_session", "wrong_frame", "navigated", "cancelled", "no_chooser"]) {
  test(`MP-08/MP-10/MP-11 visible chooser ${scenario} binds only the observed frame`, async t => {
    const root = await mkdtemp(path.join(tmpdir(), "chariox-b207-chooser-test-")); t.after(() => rm(root, { recursive: true, force: true }));
    const file = path.join(root, "report.txt"); await writeFile(file, "hi");
    const calls = []; let loader = "doc-a"; const abort = new AbortController();
    const connection = {
      async send(method, params) {
        calls.push({ method, params });
        if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "frame-a", loaderId: loader } } };
        if (method === "DOM.resolveNode") return { object: { objectId: params.backendNodeId === 1 ? "chooser" : "input" } };
        if (method === "Runtime.callFunctionOn") return { result: { value: "invalid" } };
        return {};
      },
      waitForEvent() { return { cancel() {}, promise: Promise.resolve(scenario === "no_chooser" ? null : { sessionId: scenario === "wrong_session" ? "session-b" : "session-a", params: { frameId: scenario === "wrong_frame" ? "frame-b" : "frame-a", backendNodeId: 2, mode: "selectSingle" } }) }; },
    };
    const upload = uploadBrowserFiles({ connection, sessionId: "session-a", targetId: "tab-a", documentId: "doc-a", nodeRef: "backend:1",
      filePaths: [file], uploadRoots: [root], signal: abort.signal,
      stageUploads: async ({ files }) => ({ files, markExposed() {}, discard() {} }),
      clickChooser: async ({ withInput }) => withInput(async () => { if (scenario === "navigated") loader = "doc-b"; if (scenario === "cancelled") abort.abort(); }),
    });
    if (scenario === "matching") await upload; else await assert.rejects(upload);
    assert.equal(calls.filter(call => call.method === "DOM.setFileInputFiles").length, scenario === "matching" ? 1 : 0);
    assert.deepEqual(calls.filter(call => call.method === "Page.setInterceptFileChooserDialog").at(-1).params, { enabled: false });
  });
}
