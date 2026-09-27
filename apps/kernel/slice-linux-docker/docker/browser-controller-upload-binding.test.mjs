import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, realpath, rm, symlink, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import nodeTest from "node:test";
import { execFileSync } from "node:child_process";
import { uploadBrowserFiles } from "./browser-controller-files.mjs";
import { BrowserUploadStaging, uploadCopyTimeoutMs } from "./browser-controller-upload-staging.mjs";

// Procfs identity and kernel-held flock are Linux runtime contracts.
const test = (name, run) => nodeTest(name, { skip: process.platform !== "linux" }, run);
const platformFixture = {};

async function approvedMetadata(file) {
  const metadata = await stat(file, { bigint: true });
  return { ...metadata, size: Number(metadata.size) };
}

nodeTest("upload copy deadline scales with approved bytes and remains bounded", () => {
  assert.equal(uploadCopyTimeoutMs(0), 5000);
  assert.equal(uploadCopyTimeoutMs(4 * 1024 * 1024), 6000);
  assert.equal(uploadCopyTimeoutMs(256 * 1024 * 1024), 69000);
  assert.equal(uploadCopyTimeoutMs(512 * 1024 * 1024), 120000);
});

test("upload cannot expose an outside-root replacement during renderer awaits", async (t) => {
  const root = await realpath(await mkdtemp(path.join(tmpdir(), "chariox-upload-binding-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const allowed = path.join(root, "allowed");
  await mkdir(allowed);
  const selected = path.join(allowed, "report.txt");
  const denied = path.join(root, "outside.txt");
  await writeFile(selected, "approved");
  await writeFile(denied, "outside-root-fixture");
  let consumed;
  const connection = {
    browserInstanceId: "ws://127.0.0.1:9222/devtools/browser/browser-a",
    async send(method, params) {
      if (method === "Page.getFrameTree") return { frameTree: { frame: { loaderId: "doc-a" } } };
      if (method === "DOM.resolveNode") {
        await rm(selected);
        await symlink(denied, selected);
        return { object: { objectId: "input-a" } };
      }
      if (method === "Runtime.callFunctionOn") return { result: { value: "file" } };
      if (method === "DOM.setFileInputFiles") consumed = await readFile(params.files[0], "utf8");
      return {};
    },
  };
  const result = await uploadBrowserFiles({ connection, sessionId: "session-a", targetId: "target-a",
    documentId: "doc-a", nodeRef: "backend:1", filePaths: [selected], uploadRoots: [allowed],
    stageUploads: options => new BrowserUploadStaging({ root: path.join(root, "staging"), ...platformFixture }).prepare(options) });
  assert.equal(consumed, "approved");
  assert.equal(result.total_bytes, Buffer.byteLength(consumed));
});

async function fixture(t, limits = {}) {
  const root = await realpath(await mkdtemp(path.join(tmpdir(), "chariox-upload-staging-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const file = path.join(root, "report.txt");
  await writeFile(file, "approved");
  const options = { root: path.join(root, "staging"), ...platformFixture, ...limits };
  return { root, file, options, async prepare(identity = "browser-a", extra = {}) {
    return new BrowserUploadStaging(options).prepare({ files: [file], metadata: [await approvedMetadata(file)],
      browserIdentity: `ws://127.0.0.1:9222/devtools/browser/${identity}`, ...extra });
  } };
}

test("exposed bytes survive replacement, restart and identity churn within a durable quota", async t => {
  const fixtureData = await fixture(t, { maximumBytes: 8 });
  const lease = await fixtureData.prepare();
  await lease.markExposed();
  await lease.discard();
  await writeFile(fixtureData.file, "modified");
  assert.equal(await readFile(lease.files[0], "utf8"), "approved");
  assert.equal(path.basename(lease.files[0]), "report.txt");
  assert.equal((await stat(lease.files[0])).mode & 0o777, 0o400);
  for (const identity of ["browser-a", "browser-b"]) {
    await assert.rejects(fixtureData.prepare(identity), /quota is full/);
  }
  assert.equal(await readFile(lease.files[0], "utf8"), "approved");
});

test("unexposed reservations are removed and release durable file quota", async t => {
  const fixtureData = await fixture(t, { maximumFiles: 1 });
  const first = await fixtureData.prepare();
  await assert.rejects(fixtureData.prepare("browser-b"), /quota is full/);
  await first.discard();
  await assert.rejects(stat(first.files[0]), { code: "ENOENT" });
  const second = await fixtureData.prepare("browser-b");
  await second.discard();
});

test("source replacement before descriptor opening fails and releases reservation", async t => {
  const fixtureData = await fixture(t);
  const metadata = [await approvedMetadata(fixtureData.file)];
  await rm(fixtureData.file);
  const outside = path.join(fixtureData.root, "outside.txt");
  await writeFile(outside, "outside");
  await symlink(outside, fixtureData.file);
  await assert.rejects(fixtureData.prepare("browser-a", { metadata }), /staging failed/);
  assert.deepEqual(JSON.parse(await readFile(path.join(fixtureData.options.root, "ledger.json"), "utf8")), []);
});

test("upload authorization compares exact nanoseconds without float millisecond rounding", async t => {
  const data = await fixture(t);
  const approved = await approvedMetadata(data.file);
  const lease = await data.prepare("browser-a", { metadata: [{ ...approved, mtimeMs: Number(approved.mtimeMs) + 1 }] });
  assert.equal(await readFile(lease.files[0], "utf8"), "approved");
  await lease.discard();
  await assert.rejects(data.prepare("browser-a", { metadata: [{ ...approved, mtimeNs: approved.mtimeNs + 1n }] }), /source changed/);
});

test("staged timestamps preserve exact descriptor nanoseconds at millisecond boundaries", async t => {
  const data = await fixture(t);
  for (const ns of ["1790533992267000000", "1790533992267123456"]) {
    execFileSync("python3", ["-c", "import os,sys; n=int(sys.argv[2]); os.utime(sys.argv[1], ns=(n,n))", data.file, ns]);
    const lease = await data.prepare();
    assert.equal((await stat(lease.files[0], { bigint: true })).mtimeNs, BigInt(ns));
    await lease.discard();
  }
});

test("cancellation and copy deadline fail before native dispatch", async t => {
  const fixtureData = await fixture(t);
  const abort = new AbortController();
  abort.abort();
  await assert.rejects(fixtureData.prepare("browser-a", { signal: abort.signal }), { code: "browser_action_cancelled" });
  let tick = 0;
  fixtureData.options.now = () => (tick += 5001);
  await assert.rejects(fixtureData.prepare(), /bounded copy deadline/);
});

test("descriptor path mismatch and an in-copy deadline remove private reservations", async t => {
  const data = await fixture(t);
  data.options.descriptorPath = async () => "/outside/report.txt";
  await assert.rejects(data.prepare(), /source path changed/);
  assert.deepEqual(JSON.parse(await readFile(path.join(data.options.root, "ledger.json"), "utf8")), []);
  Object.assign(data.options, platformFixture);
  if (process.platform === "linux") delete data.options.descriptorPath;
  let tick = 0;
  data.options.now = () => (tick += 1001);
  await assert.rejects(data.prepare(), /bounded copy deadline/);
  assert.deepEqual(JSON.parse(await readFile(path.join(data.options.root, "ledger.json"), "utf8")), []);
});

test("upload staging is included in the slice image, recovery overlay and native source tree", async () => {
  const name = "browser-controller-upload-staging.mjs";
  const dockerfile = await readFile(new URL("./Dockerfile", import.meta.url), "utf8");
  const provisioner = await readFile(new URL("../provision-linux-docker-slice.sh", import.meta.url), "utf8");
  const packager = await readFile(new URL("../../../../scripts/package-managed-kernel-release.mjs", import.meta.url), "utf8");
  assert.ok(dockerfile.includes(`docker/${name} /opt/chariox-slice/${name}`));
  assert.ok(provisioner.includes(`docker/${name}" "$SLICE_NAME:/opt/chariox-slice/${name}"`));
  assert.match(packager, /const SLICE_BUILD_CONTEXT_SOURCES = \[[\s\S]*?"apps\/kernel"/);
  for (const helper of ["browser-lifecycle.py", "browser-upload-store.py"]) {
    assert.ok(dockerfile.includes(`docker/${helper} /opt/chariox-slice/${helper}`));
    assert.ok(provisioner.includes(`docker/${helper}" "$SLICE_NAME:/opt/chariox-slice/${helper}"`));
  }
});

for (const phase of ["DOM.resolveNode", "DOM.setFileInputFiles"]) {
  test(`upload retains bytes only once native dispatch may have occurred: ${phase}`, async t => {
    const data = await fixture(t);
    let staged;
    const connection = {
      browserInstanceId: "ws://127.0.0.1:9222/devtools/browser/browser-a",
      async send(method, params) {
        if (method === phase) throw Object.assign(new Error("reply lost"), { code: "browser_cdp_command_failed" });
        if (method === "Page.getFrameTree") return { frameTree: { frame: { loaderId: "doc-a" } } };
        if (method === "DOM.resolveNode") return { object: { objectId: "input-a" } };
        if (method === "Runtime.callFunctionOn") {
          assert.doesNotMatch(params.functionDeclaration, /dispatchEvent|DataTransfer/);
          return { result: { value: "file" } };
        }
        return {};
      },
    };
    await assert.rejects(uploadBrowserFiles({ connection, sessionId: "s", targetId: "t", documentId: "doc-a",
      nodeRef: "backend:1", filePaths: [data.file], uploadRoots: [data.root],
      stageUploads: async options => { const lease = await new BrowserUploadStaging(data.options).prepare(options); staged = lease.files[0]; return lease; },
    }), { code: "stale_element_reference" });
    if (phase === "DOM.resolveNode") await assert.rejects(stat(staged), { code: "ENOENT" });
    else assert.equal(await readFile(staged, "utf8"), "approved");
  });
}
