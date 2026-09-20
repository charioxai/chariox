import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserPageFeatureError,
  BrowserPageFeatures,
  PAGE_FEATURE_ERROR_CODES,
} from "./browser-controller-page-features.mjs";

const ELEMENT = Object.freeze({
  tab_id: "tab-7",
  document_id: "document-3",
  snapshot_revision: 4,
  backend_node_id: 91,
});

class FakeConnection {
  constructor(responses = {}) {
    this.responses = responses;
    this.calls = [];
  }

  async send(method, params) {
    this.calls.push({ method, params });
    const response = this.responses[method];
    if (response instanceof Error) throw response;
    return typeof response === "function" ? response(params) : response ?? {};
  }
}

function fakeTime() {
  let value = 0;
  return {
    now: () => value,
    sleep: async (milliseconds) => {
      value += milliseconds;
    },
  };
}

function fakeFilesystem() {
  const resolved = new Map([
    ["/safe", "/safe"],
    ["/safe/downloads", "/safe/downloads"],
    ["/safe/report.txt", "/safe/report.txt"],
    ["/safe/large.txt", "/safe/large.txt"],
    ["/safe/link.txt", "/private/secret.txt"],
    ["/private/secret.txt", "/private/secret.txt"],
  ]);
  return {
    realpath: async (candidate) => {
      if (!resolved.has(candidate)) throw new Error("missing");
      return resolved.get(candidate);
    },
    stat: async (candidate) => ({
      isDirectory: () => candidate === "/safe" || candidate === "/safe/downloads",
      isFile: () => candidate.endsWith(".txt"),
      size: candidate === "/safe/large.txt" ? 64 * 1024 * 1024 + 1 : 1024,
    }),
  };
}

function assertCode(code) {
  return (error) => error instanceof BrowserPageFeatureError && error.code === code;
}

test("describes frame and shadow context without returning page content", async () => {
  const connection = new FakeConnection({
    "DOM.describeNode": {
      node: {
        frameId: "frame-2",
        shadowRootType: "open",
        nodeName: "INPUT",
        localName: "input",
        nodeValue: "secret page value",
        attributes: ["value", "secret"],
      },
    },
  });
  const features = new BrowserPageFeatures();
  const result = await features.describeElement({ connection, element: ELEMENT });

  assert.deepEqual(result, {
    tab_id: "tab-7",
    document_id: "document-3",
    snapshot_revision: 4,
    frame_id: "frame-2",
    shadow_root_type: "open",
    node_name: "INPUT",
    local_name: "input",
  });
  assert.doesNotMatch(JSON.stringify(result), /secret/);
  assert.deepEqual(connection.calls[0], {
    method: "DOM.describeNode",
    params: { backendNodeId: 91, depth: 0, pierce: true },
  });
});

test("accepts or dismisses dialogs without returning prompt text", async () => {
  const connection = new FakeConnection();
  const features = new BrowserPageFeatures();
  const accepted = await features.handleDialog({
    connection,
    dialog: { action: "accept", prompt_text: "private response" },
  });
  const dismissed = await features.handleDialog({
    connection,
    dialog: { action: "dismiss" },
  });

  assert.deepEqual(accepted, { action: "accept" });
  assert.deepEqual(dismissed, { action: "dismiss" });
  assert.doesNotMatch(JSON.stringify([accepted, dismissed]), /private response/);
  assert.deepEqual(connection.calls, [
    {
      method: "Page.handleJavaScriptDialog",
      params: { accept: true, promptText: "private response" },
    },
    {
      method: "Page.handleJavaScriptDialog",
      params: { accept: false },
    },
  ]);
});

test("waits for a stable opaque popup tab and bounds the wait", async () => {
  const time = fakeTime();
  const snapshots = [
    { tabs: [{ tab_id: "tab-1", target_generation: 1 }] },
    { tabs: [{ tab_id: "tab-1", target_generation: 1 }, { tab_id: "tab-2", target_generation: 1 }] },
  ];
  const features = new BrowserPageFeatures(time);
  const result = await features.waitForPopup({
    known_tab_ids: ["tab-1"],
    timeout_ms: 500,
    listTabs: async () => snapshots.shift(),
  });
  assert.deepEqual(result, {
    tab_id: "tab-2",
    target_generation: 1,
    attempts: 2,
    elapsed_ms: 50,
  });

  const timeout = fakeTime();
  const timingOut = new BrowserPageFeatures(timeout);
  await assert.rejects(
    timingOut.waitForPopup({
      known_tab_ids: ["tab-1"],
      timeout_ms: 100,
      listTabs: async () => ({ tabs: [{ tab_id: "tab-1", target_generation: 1 }] }),
    }),
    assertCode(PAGE_FEATURE_ERROR_CODES.TIMEOUT),
  );
});

test("configures downloads only inside the controller-owned root", async () => {
  const filesystem = fakeFilesystem();
  const connection = new FakeConnection();
  const features = new BrowserPageFeatures({
    ...filesystem,
    downloadRoot: "/safe/downloads",
  });
  assert.deepEqual(await features.configureDownloads({ connection }), { enabled: true });
  assert.deepEqual(connection.calls[0], {
    method: "Browser.setDownloadBehavior",
    params: {
      behavior: "allowAndName",
      downloadPath: "/safe/downloads",
      eventsEnabled: true,
    },
  });

  const disabled = new BrowserPageFeatures(filesystem);
  await assert.rejects(
    disabled.configureDownloads({ connection: new FakeConnection() }),
    assertCode(PAGE_FEATURE_ERROR_CODES.PATH_DENIED),
  );
});

test("uploads regular files from allowed roots without returning their paths", async () => {
  const filesystem = fakeFilesystem();
  const connection = new FakeConnection();
  const features = new BrowserPageFeatures({
    ...filesystem,
    uploadRoots: ["/safe"],
  });
  const result = await features.uploadFiles({
    connection,
    element: ELEMENT,
    paths: ["/safe/report.txt"],
  });
  assert.deepEqual(result, {
    tab_id: "tab-7",
    document_id: "document-3",
    snapshot_revision: 4,
    file_count: 1,
  });
  assert.doesNotMatch(JSON.stringify(result), /report\.txt/);
  assert.deepEqual(connection.calls[0], {
    method: "DOM.setFileInputFiles",
    params: { backendNodeId: 91, files: ["/safe/report.txt"] },
  });
});

test("rejects traversal, symlink escape, missing, and non-regular upload files before CDP", async () => {
  const filesystem = fakeFilesystem();
  const features = new BrowserPageFeatures({
    ...filesystem,
    uploadRoots: ["/safe"],
  });
  for (const candidate of [
    "relative.txt",
    "/safe/../private/secret.txt",
    "/safe/link.txt",
    "/safe/missing.txt",
    "/safe/downloads",
    "/safe/large.txt",
  ]) {
    const connection = new FakeConnection();
    await assert.rejects(
      features.uploadFiles({ connection, element: ELEMENT, paths: [candidate] }),
      assertCode(PAGE_FEATURE_ERROR_CODES.PATH_DENIED),
    );
    assert.equal(connection.calls.length, 0);
  }

  const bulkConnection = new FakeConnection();
  const bulkFeatures = new BrowserPageFeatures({
    uploadRoots: ["/safe"],
    realpath: async (candidate) => candidate,
    stat: async (candidate) => ({
      isDirectory: () => candidate === "/safe",
      isFile: () => candidate !== "/safe",
      size: candidate === "/safe" ? 0 : 20 * 1024 * 1024,
    }),
  });
  await assert.rejects(
    bulkFeatures.uploadFiles({
      connection: bulkConnection,
      element: ELEMENT,
      paths: Array.from({ length: 16 }, (_, index) => `/safe/bulk-${index}.bin`),
    }),
    assertCode(PAGE_FEATURE_ERROR_CODES.PATH_DENIED),
  );
  assert.equal(bulkConnection.calls.length, 0);
});

test("grants only bounded origin-scoped permissions and can reset them", async () => {
  const connection = new FakeConnection();
  const features = new BrowserPageFeatures();
  const granted = await features.grantPermissions({
    connection,
    origin: "https://example.test",
    permissions: ["notifications", "clipboardReadWrite", "notifications"],
  });
  assert.deepEqual(granted, {
    origin: "https://example.test",
    permissions: ["clipboardReadWrite", "notifications"],
  });
  assert.deepEqual(await features.resetPermissions({ connection }), { reset: true });
  assert.deepEqual(connection.calls, [
    {
      method: "Browser.grantPermissions",
      params: {
        origin: "https://example.test",
        permissions: ["clipboardReadWrite", "notifications"],
      },
    },
    { method: "Browser.resetPermissions", params: {} },
  ]);
});

test("rejects credentialed, path-bearing, and unsupported permission requests before CDP", async () => {
  const features = new BrowserPageFeatures();
  for (const request of [
    { origin: "https://user:pass@example.test", permissions: ["notifications"] },
    { origin: "https://example.test/path", permissions: ["notifications"] },
    { origin: "file:///tmp/page", permissions: ["notifications"] },
    { origin: "https://example.test", permissions: ["unknownPermission"] },
  ]) {
    const connection = new FakeConnection();
    await assert.rejects(
      features.grantPermissions({ connection, ...request }),
      assertCode(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT),
    );
    assert.equal(connection.calls.length, 0);
  }
});
