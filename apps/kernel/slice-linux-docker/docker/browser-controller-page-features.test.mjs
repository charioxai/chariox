import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rename, rm, stat as nodeStat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";

import {
  BrowserPageFeatureError,
  BrowserPageFeatures,
  CHROMIUM_PERMISSION_TYPES,
  PAGE_FEATURE_ERROR_CODES,
} from "./browser-controller-page-features.mjs";

const ELEMENT = Object.freeze({
  tab_id: "tab-7",
  document_id: "document-3",
  snapshot_revision: 4,
  backend_node_id: 91,
  frame_id: "frame-main",
  main_frame_id: "frame-main",
});

const EXPECTED_CHROMIUM_PERMISSION_TYPES = Object.freeze([
  "ar",
  "audioCapture",
  "automaticFullscreen",
  "backgroundFetch",
  "backgroundSync",
  "cameraPanTiltZoom",
  "capturedSurfaceControl",
  "clipboardReadWrite",
  "clipboardSanitizedWrite",
  "displayCapture",
  "durableStorage",
  "geolocation",
  "handTracking",
  "idleDetection",
  "keyboardLock",
  "localFonts",
  "localNetwork",
  "localNetworkAccess",
  "loopbackNetwork",
  "midi",
  "midiSysex",
  "nfc",
  "notifications",
  "paymentHandler",
  "periodicBackgroundSync",
  "pointerLock",
  "protectedMediaIdentifier",
  "sensors",
  "smartCard",
  "speakerSelection",
  "storageAccess",
  "topLevelStorageAccess",
  "videoCapture",
  "vr",
  "wakeLockScreen",
  "wakeLockSystem",
  "webAppInstallation",
  "webPrinting",
  "windowManagement",
]);

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
      dev: 1,
      ino: candidate === "/safe" ? 10 : candidate === "/safe/downloads" ? 11 : 20,
    }),
    stageUpload: async ({ destinationPath, expectedSize }) => ({
      path: destinationPath,
      size: expectedSize,
    }),
  };
}

function fakeUploadLeaseFilesystem() {
  const filesystem = fakeFilesystem();
  let stageNumber = 0;
  const staged = new Map();
  const removedStages = [];
  const pendingTimers = new Set();
  filesystem.mkdtemp = async () => `/tmp/chariox-browser-upload-test-${++stageNumber}`;
  filesystem.remove = async (stageDirectory) => {
    removedStages.push(stageDirectory);
    for (const candidate of staged.keys()) {
      if (candidate.startsWith(`${stageDirectory}/`)) staged.delete(candidate);
    }
  };
  filesystem.setTimeoutFn = (callback, delay) => {
    const timer = { callback, delay, unref() {} };
    pendingTimers.add(timer);
    return timer;
  };
  filesystem.clearTimeoutFn = (timer) => pendingTimers.delete(timer);
  filesystem.stageUpload = async ({ destinationPath, expectedSize }) => {
    staged.set(destinationPath, Buffer.alloc(expectedSize, 0x41));
    return { path: destinationPath, size: expectedSize };
  };
  filesystem.staged = staged;
  filesystem.removedStages = removedStages;
  filesystem.pendingTimers = pendingTimers;
  return filesystem;
}

function assertCode(code) {
  return (error) => error instanceof BrowserPageFeatureError && error.code === code;
}

test("describes frame and shadow context without returning page content", async () => {
  const connection = new FakeConnection({
    "DOM.describeNode": {
      node: {
        nodeName: "INPUT",
        localName: "input",
        nodeValue: "secret page value",
        attributes: ["value", "secret"],
      },
    },
  });
  const features = new BrowserPageFeatures();
  const result = await features.describeElement({
    connection,
    element: {
      ...ELEMENT,
      frame_id: "frame-child",
      main_frame_id: "frame-main",
      shadow_root_type: "open",
    },
  });

  assert.deepEqual(result, {
    tab_id: "tab-7",
    document_id: "document-3",
    snapshot_revision: 4,
    frame_id: "frame-child",
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
  const connection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-3" } } },
  });
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
  const setFilesCall = connection.calls.find(({ method }) => method === "DOM.setFileInputFiles");
  assert.ok(setFilesCall);
  assert.deepEqual(setFilesCall, {
    method: "DOM.setFileInputFiles",
    params: { backendNodeId: 91, files: [setFilesCall.params.files[0]] },
  });
  assert.notEqual(setFilesCall.params.files[0], "/safe/report.txt");
  assert.match(setFilesCall.params.files[0], /chariox-browser-upload-/);
});

test("retains staged bytes for page consumption after return and cleans them at lease expiry", async () => {
  const filesystem = fakeUploadLeaseFilesystem();
  const connection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-3" } } },
  });
  const features = new BrowserPageFeatures({
    ...filesystem,
    uploadRoots: ["/safe"],
  });

  await features.uploadFiles({ connection, element: ELEMENT, paths: ["/safe/report.txt"] });
  const stagedPath = connection.calls.find(({ method }) => method === "DOM.setFileInputFiles")
    .params.files[0];
  const stagedBytes = filesystem.staged.get(stagedPath);
  assert.ok(stagedBytes);
  assert.deepEqual(stagedBytes, Buffer.alloc(1024, 0x41));
  assert.equal(filesystem.removedStages.length, 0);

  // The page can read and submit the file after the action has returned.
  const readAndSubmit = (candidate) => {
    const bytes = filesystem.staged.get(candidate);
    assert.ok(bytes);
    return { submitted: true, bytes };
  };
  assert.deepEqual(readAndSubmit(stagedPath), { submitted: true, bytes: stagedBytes });
  const [lease] = filesystem.pendingTimers;
  assert.equal(lease.delay, 60_000);
  await lease.callback();
  assert.equal(filesystem.staged.has(stagedPath), false);
  assert.equal(filesystem.removedStages.length, 1);
});

test("copies bytes from an identity-bound descriptor across an adversarial pathname swap", async (t) => {
  const fixtureRoot = await mkdtemp(path.join(tmpdir(), "chariox-page-upload-swap-"));
  const uploadRoot = path.join(fixtureRoot, "safe");
  const secretRoot = path.join(fixtureRoot, "secret");
  const sourcePath = path.join(uploadRoot, "report.txt");
  const secretPath = path.join(secretRoot, "report.txt");
  const sourceBytes = Buffer.from("approved upload bytes");
  const secretBytes = Buffer.from("attacker bytes");
  await mkdir(uploadRoot);
  await mkdir(secretRoot);
  await writeFile(sourcePath, sourceBytes);
  await writeFile(secretPath, secretBytes);
  t.after(() => rm(fixtureRoot, { recursive: true, force: true }));

  let rootStatCalls = 0;
  let swapped = false;
  const stat = async (candidate) => {
    const metadata = await nodeStat(candidate);
    if (candidate === uploadRoot && ++rootStatCalls === 3 && !swapped) {
      await rename(sourcePath, `${sourcePath}.original`);
      await symlink(secretPath, sourcePath);
      swapped = true;
    }
    return metadata;
  };
  const pendingTimers = new Set();
  const connection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-3" } } },
  });
  const features = new BrowserPageFeatures({
    uploadRoots: [uploadRoot],
    stat,
    mkdtemp: async (prefix) => mkdtemp(prefix),
    setTimeoutFn: (callback, delay) => {
      const timer = { callback, delay, unref() {} };
      pendingTimers.add(timer);
      return timer;
    },
    clearTimeoutFn: (timer) => pendingTimers.delete(timer),
  });

  await features.uploadFiles({
    connection,
    element: ELEMENT,
    paths: [sourcePath],
  });
  assert.equal(swapped, true);
  const stagedPath = connection.calls.find(({ method }) => method === "DOM.setFileInputFiles")
    .params.files[0];
  const stagedBytes = await readFile(stagedPath);
  assert.deepEqual(stagedBytes, sourceBytes);
  assert.notDeepEqual(stagedBytes, secretBytes);
  const [lease] = pendingTimers;
  await lease.callback();
});

test("keeps configured roots fixed and rejects source swaps or staged growth", async () => {
  const rootReplacementStat = async (candidate) => ({
    isDirectory: () => candidate === "/safe",
    isFile: () => candidate === "/safe/report.txt",
    size: 1024,
    dev: candidate === "/safe" ? 1 : 1,
    ino: candidate === "/safe" ? (rootReplacementStat.calls++ === 0 ? 10 : 11) : 20,
  });
  rootReplacementStat.calls = 0;
  const rootReplacement = new BrowserPageFeatures({
    uploadRoots: ["/safe"],
    realpath: async (candidate) => candidate,
    stat: rootReplacementStat,
    stageUpload: async ({ destinationPath, expectedSize }) => ({
      path: destinationPath,
      size: expectedSize,
    }),
  });
  await rootReplacement.prepare();
  const replacementConnection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-3" } } },
  });
  await assert.rejects(
    rootReplacement.uploadFiles({
      connection: replacementConnection,
      element: ELEMENT,
      paths: ["/safe/report.txt"],
    }),
    assertCode(PAGE_FEATURE_ERROR_CODES.PATH_DENIED),
  );
  assert.equal(replacementConnection.calls.length, 0);

  let filePathLookups = 0;
  const swappedPath = new BrowserPageFeatures({
    uploadRoots: ["/safe"],
    realpath: async (candidate) => {
      if (candidate === "/safe/report.txt") {
        filePathLookups += 1;
        return filePathLookups === 1 ? candidate : "/private/secret.txt";
      }
      return candidate;
    },
    stat: async (candidate) => ({
      isDirectory: () => candidate === "/safe",
      isFile: () => candidate === "/safe/report.txt",
      size: 1024,
      dev: candidate === "/safe" ? 1 : 1,
      ino: candidate === "/safe" ? 10 : 20,
    }),
    stageUpload: async ({ destinationPath, expectedSize }) => ({
      path: destinationPath,
      size: expectedSize,
    }),
  });
  const swapConnection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-3" } } },
  });
  await assert.rejects(
    swappedPath.uploadFiles({
      connection: swapConnection,
      element: ELEMENT,
      paths: ["/safe/report.txt"],
    }),
    assertCode(PAGE_FEATURE_ERROR_CODES.PATH_DENIED),
  );
  assert.equal(swapConnection.calls.length, 0);

  const grownStage = new BrowserPageFeatures({
    uploadRoots: ["/safe"],
    realpath: async (candidate) => candidate,
    stat: async (candidate) => ({
      isDirectory: () => candidate === "/safe",
      isFile: () => candidate === "/safe/report.txt",
      size: 1024,
      dev: candidate === "/safe" ? 1 : 1,
      ino: candidate === "/safe" ? 10 : 20,
    }),
    stageUpload: async ({ destinationPath }) => ({
      path: destinationPath,
      size: 64 * 1024 * 1024 + 1,
    }),
  });
  const growthConnection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-3" } } },
  });
  await assert.rejects(
    grownStage.uploadFiles({
      connection: growthConnection,
      element: ELEMENT,
      paths: ["/safe/report.txt"],
    }),
    assertCode(PAGE_FEATURE_ERROR_CODES.PATH_DENIED),
  );
  assert.equal(growthConnection.calls.length, 0);
});

test("returns STALE_DOCUMENT when navigation wins the final upload check", async () => {
  const connection = new FakeConnection({
    "Page.getFrameTree": { frameTree: { frame: { loaderId: "document-4" } } },
  });
  const features = new BrowserPageFeatures({
    ...fakeFilesystem(),
    uploadRoots: ["/safe"],
  });
  await assert.rejects(
    features.uploadFiles({ connection, element: ELEMENT, paths: ["/safe/report.txt"] }),
    assertCode(PAGE_FEATURE_ERROR_CODES.STALE_DOCUMENT),
  );
  assert.deepEqual(connection.calls.map(({ method }) => method), ["Page.getFrameTree"]);
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
      dev: 1,
      ino: candidate === "/safe" ? 10 : 20,
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

test("accepts every permission entry from the pinned Chromium protocol", async () => {
  assert.deepEqual([...CHROMIUM_PERMISSION_TYPES], EXPECTED_CHROMIUM_PERMISSION_TYPES);
  assert.equal(new Set(CHROMIUM_PERMISSION_TYPES).size, EXPECTED_CHROMIUM_PERMISSION_TYPES.length);
  const connection = new FakeConnection();
  const features = new BrowserPageFeatures();
  const granted = await features.grantPermissions({
    connection,
    origin: "https://example.test",
    permissions: EXPECTED_CHROMIUM_PERMISSION_TYPES,
  });
  assert.deepEqual(granted.permissions, EXPECTED_CHROMIUM_PERMISSION_TYPES);
  assert.deepEqual(connection.calls, [{
    method: "Browser.grantPermissions",
    params: {
      origin: "https://example.test",
      permissions: EXPECTED_CHROMIUM_PERMISSION_TYPES,
    },
  }]);
});

test("rejects credentialed, path-bearing, and unsupported permission requests before CDP", async () => {
  const features = new BrowserPageFeatures();
  for (const request of [
    { origin: "https://user:pass@example.test", permissions: ["notifications"] },
    { origin: "https://example.test/path", permissions: ["notifications"] },
    { origin: "file:///tmp/page", permissions: ["notifications"] },
    { origin: "https://example.test", permissions: ["unknownPermission"] },
    { origin: "https://example.test", permissions: ["accessibilityEvents"] },
    { origin: "https://example.test", permissions: ["videoCapturePanTiltZoom"] },
  ]) {
    const connection = new FakeConnection();
    await assert.rejects(
      features.grantPermissions({ connection, ...request }),
      assertCode(PAGE_FEATURE_ERROR_CODES.INVALID_ARGUMENT),
    );
    assert.equal(connection.calls.length, 0);
  }
});
