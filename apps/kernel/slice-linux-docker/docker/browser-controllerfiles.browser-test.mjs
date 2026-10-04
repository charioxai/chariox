// MP-08/MP-10/MP-11: actual Chromium/controller fixture evidence only.
// Kernel-owned Browser artifact delivery and provider/Web/TUI conjunction are
// deliberately separate acceptance gates; this test cannot establish them.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, readFile, readdir, realpath, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { createHash } from "node:crypto";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { handleBrowserControllerRequest } from "./browser-controller.mjs";
import { BrowserUploadStaging } from "./browser-controller-upload-staging.mjs";
import { controllerfilesSamples, assertControllerfilesReceipt, startControllerfilesFixture } from "../../../cli/scripts/lib/browser-controllerfiles-fixture.mjs";

const viewport = { css_width: 960, css_height: 640, device_scale_factor: 1,
  desktop_pixel_width: 960, desktop_pixel_height: 640 };
const lifecycle = fileURLToPath(new URL("./browser-lifecycle.py", import.meta.url));

test("MP-08/MP-10/MP-11 actual CDP upload/download and passive capture fixture", { timeout: 45000 }, async t => {
  assert.ok(process.env.CHARIOX_TEST_CHROMIUM, "explicit installed Chromium path required; no browser downloads");
  let evidence;
  if (process.env.CHARIOX_CONTROLLERFILES_EVIDENCE) {
    assert.ok(path.isAbsolute(process.env.CHARIOX_CONTROLLERFILES_EVIDENCE), "absolute external evidence directory required");
    evidence = await realpath(process.env.CHARIOX_CONTROLLERFILES_EVIDENCE);
    const repository = await realpath(fileURLToPath(new URL("../../../../", import.meta.url)));
    const relative = path.relative(repository, evidence);
    assert.ok(relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative), "evidence must remain outside the repository");
    assert.ok((await stat(evidence)).isDirectory());
    assert.equal((await stat(evidence)).uid, process.getuid(), "evidence must belong to the executing user");
    assert.deepEqual(await readdir(evidence), [], "use a fresh evidence directory for each exact source run");
  }
  const root = await realpath(await mkdtemp(path.join(tmpdir(), "chariox-b207-controllerfiles-")));
  const profile = path.join(root, "profile");
  const oldLifecycleRoot = process.env.CHARIOX_BROWSER_LIFECYCLE_ROOT;
  process.env.CHARIOX_BROWSER_LIFECYCLE_ROOT = path.join(root, "lifetimes");
  let browser;
  let launched = false;
  const fixture = await startControllerfilesFixture();
  t.after(async () => {
    try { await browser?.close(); }
    finally {
      try { if (launched) execFileSync("python3", [lifecycle, "stop", profile], { timeout: 15000, stdio: "pipe" }); }
      finally {
        await fixture.close();
        await rm(root, { recursive: true, force: true });
        if (oldLifecycleRoot === undefined) delete process.env.CHARIOX_BROWSER_LIFECYCLE_ROOT;
        else process.env.CHARIOX_BROWSER_LIFECYCLE_ROOT = oldLifecycleRoot;
      }
    }
  });
  execFileSync("python3", [lifecycle, "start", profile, path.join(root, "browser.log"),
    process.env.CHARIOX_TEST_CHROMIUM, "--headless=new", "--remote-debugging-port=0",
    `--user-data-dir=${profile}`, "--disable-dev-shm-usage", "--no-first-run", "about:blank"],
  { timeout: 10000, stdio: "pipe" });
  launched = true;
  const port = await until(async () => {
    try { return Number((await readFile(path.join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]); }
    catch { return null; }
  });
  assert.ok(Number.isSafeInteger(port) && port > 0);
  const uploads = path.join(root, "uploads");
  const downloads = path.join(root, "downloads");
  await mkdir(uploads);
  const expected = controllerfilesSamples();
  for (const file of expected) await writeFile(path.join(uploads, file.name), file.bytes);
  browser = new BrowserCdpClient({ debuggerEndpoint: `http://127.0.0.1:${port}`,
    uploadRoots: [uploads], downloadDirectory: downloads,
    stageUploads: options => new BrowserUploadStaging({ root: path.join(root, "staging") }).prepare(options) });
  let id = 0;
  const request = (method, params) => handleBrowserControllerRequest({ id: ++id, method, params }, { browser });
  const reconcile = async () => {
    const reply = await request("browser.reconcile", { viewport });
    assert.equal(reply.ok, true, reply.error?.code);
    return reply.result;
  };
  const initial = (await reconcile()).tabs[0];
  assert.equal((await request("browser.navigate", { ...initial, url: fixture.origin })).ok, true);
  const current = (await reconcile()).tabs.find(tab => tab.target_id === initial.target_id);
  const { connection, sessionId } = await browser.resolvePageTarget(current.target_id);
  const evaluate = async expression => {
    const reply = await connection.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }, sessionId);
    assert.equal(reply.exceptionDetails, undefined, "fixture evaluation failed");
    return reply.result.value;
  };
  // The raw fixture observer supplies only verifier state, never tool references.
  await until(async () => evaluate("Boolean(document.querySelector('#choose'))"));
  const snapshot = await request("browser.snapshot", current);
  assert.equal(snapshot.ok, true, snapshot.error?.code);
  const input = snapshot.result.dom_nodes.find(node => node.node_name === "INPUT" && node.attributes.type === "file");
  const chooser = snapshot.result.accessibility_nodes.find(node => node.role === "button" && node.name === "Choose documents");
  assert.ok(input && chooser);
  assert.equal(input.bounds, null, "fixture must exercise a hidden input behind the observed visible chooser");
  // Keep the supported input path as a control; this does not prove the chooser.
  const uploaded = await request("browser.upload", { ...current, node_ref: input.node_ref,
    file_paths: expected.map(file => path.join(uploads, file.name)) });
  assert.equal(uploaded.ok, true, uploaded.error?.code);
  const send = snapshot.result.accessibility_nodes.find(node => node.role === "button" && node.name === "Send documents");
  assert.equal((await request("browser.action", { ...current, node_ref: send.node_ref, action: { kind: "click" } })).ok, true);
  await until(async () => fixture.receipts[0]);
  assertControllerfilesReceipt(fixture.receipts[0], expected);
  const cancel = snapshot.result.accessibility_nodes.find(node => node.role === "button" && node.name === "Cancel selection");
  assert.equal((await request("browser.action", { ...current, node_ref: cancel.node_ref, action: { kind: "click" } })).ok, true);
  assert.equal(await evaluate("document.querySelector('#attachment').files.length"), 0);
  for (const file of [path.join(uploads, "missing"), path.join(root, "outside.txt")]) {
    if (file.endsWith("outside.txt")) await writeFile(file, "outside fixture");
    const denied = await request("browser.upload", { ...current, node_ref: input.node_ref, file_paths: [file] });
    assert.equal(denied.ok, false);
    assert.ok(["browser_upload_invalid", "browser_upload_denied"].includes(denied.error.code));
    assert.equal(await evaluate("document.querySelector('#attachment').files.length"), 0);
  }
  assert.equal((await request("browser.downloads.configure", current)).ok, true);
  for (const file of expected) {
    const fresh = await request("browser.snapshot", current);
    const label = { "report.txt": "Download report", "binary.bin": "Download binary", "document.pdf": "Download PDF" }[file.name];
    const link = fresh.result.accessibility_nodes.find(node => node.role === "link" && node.name === label);
    const cursor = browser.eventJournal.cursor();
    assert.equal((await request("browser.action", { ...current, node_ref: link.node_ref, action: { kind: "click" } })).ok, true);
    const progress = await until(async () => browser.pollEvents({ browser_generation: browser.browserGeneration, cursor }).events
      .find(event => event.kind === "download_progress" && event.data.state === "completed"));
    assert.equal(progress.target_id, current.target_id);
    assert.deepEqual(await readFile(path.join(downloads, progress.data.guid)), file.bytes);
    const artifact = await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration,
      viewport, kind: "download", guid: progress.data.guid });
    assert.equal(artifact.ok, true, artifact.error?.code);
    assert.deepEqual(Buffer.from(artifact.result.data_base64, "base64"), file.bytes);
    assert.equal(artifact.result.display_name, file.name);
    assert.equal(artifact.result.sha256, createHash("sha256").update(file.bytes).digest("hex"));
  }
  const slowSnapshot = await request("browser.snapshot", current);
  const slow = slowSnapshot.result.accessibility_nodes.find(node => node.role === "link" && node.name === "Download slowly");
  const downloadCursor = browser.eventJournal.cursor();
  assert.equal((await request("browser.action", { ...current, node_ref: slow.node_ref, action: { kind: "click" } })).ok, true);
  const started = await until(async () => browser.pollEvents({ browser_generation: browser.browserGeneration, cursor: downloadCursor }).events
    .find(event => event.kind === "download_started"));
  assert.equal(started.target_id, current.target_id);
  const canceled = await request("browser.downloads.cancel", { browser_generation: browser.browserGeneration, guid: started.data.guid });
  assert.equal(canceled.ok, true, canceled.error?.code);
  await until(async () => browser.pollEvents({ browser_generation: browser.browserGeneration, cursor: downloadCursor }).events
    .find(event => event.kind === "download_progress" && event.data.guid === started.data.guid && event.data.state === "canceled"));
  await until(async () => !(await readdir(downloads)).some(name => name.startsWith(started.data.guid)));
  const retired = await request("browser.downloads.cancel", { browser_generation: browser.browserGeneration, guid: started.data.guid });
  assert.equal(retired.ok, false);
  assert.equal(retired.error.code, "browser_download_not_active");
  // MP-08/MP-10/MP-11: actual Browser image bytes via the controller seam.
  // Capture actual bytes from the same controller session and pin its document
  // before/after. These bytes have not traversed the kernel/MCP image path.
  const before = (await connection.send("Page.getFrameTree", {}, sessionId)).frameTree.frame.loaderId;
  const image = Buffer.from((await connection.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false }, sessionId)).data, "base64");
  const after = (await connection.send("Page.getFrameTree", {}, sessionId)).frameTree.frame.loaderId;
  assert.equal(before, current.document_id);
  assert.equal(after, before);
  assert.deepEqual(image.subarray(0, 8), Buffer.from([137,80,78,71,13,10,26,10]));
  assert.equal(image.readUInt32BE(16), viewport.css_width);
  assert.equal(image.readUInt32BE(20), viewport.css_height);
  const cursor = browser.eventJournal.cursor();
  await evaluate("fetch('/network-proof?private=synthetic', { headers: { authorization: 'Bearer synthetic' } }).then(() => fetch('/network-proof?private=synthetic', { headers: { authorization: 'Bearer synthetic' } })).then(() => true)");
  assert.equal(fixture.network.at(-1).cookiePresent, true);
  assert.equal(fixture.network.at(-1).authPresent, true);
  const events = browser.pollEvents({ browser_generation: browser.browserGeneration, cursor }).events;
  assert.ok(events.some(event => event.kind === "network_request" && event.data.url.endsWith("/network-proof")));
  assert.ok(events.every(event => event.target_id === current.target_id));
  assert.doesNotMatch(JSON.stringify(events), /synthetic|authorization|fixture_private|set-cookie/i);
  const imageArtifact = await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration, viewport, kind: "image" });
  assert.equal(imageArtifact.ok, true, imageArtifact.error?.code);
  assert.equal(imageArtifact.result.document_id, current.document_id);
  assert.equal(imageArtifact.result.target_id, current.target_id);
  const nativeImage = Buffer.from(imageArtifact.result.data_base64, "base64");
  assert.equal(nativeImage.readUInt32BE(16), viewport.css_width);
  assert.equal(nativeImage.readUInt32BE(20), viewport.css_height);
  assert.equal(imageArtifact.result.sha256, createHash("sha256").update(nativeImage).digest("hex"));
  const networkArtifact = await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration, viewport, kind: "network" });
  assert.equal(networkArtifact.ok, true, networkArtifact.error?.code);
  const harBytes = Buffer.from(networkArtifact.result.data_base64, "base64");
  assert.doesNotMatch(harBytes.toString(), /synthetic|authorization|fixture_private|set-cookie/i);
  const networkEntries = JSON.parse(harBytes).log.entries.filter(entry => entry.request.url.endsWith("/network-proof"));
  assert.ok(networkEntries.length >= 2);
  assert.ok(networkEntries.every(entry => entry.request.extra_info_observed === true));
  assert.ok(networkEntries.every(entry => entry.request.headers.some(header => header.name === "accept" && header.value === fixture.network.at(-1).accept)));
  assert.deepEqual(imageArtifact.result.geometry, { pageX:0, pageY:0, clientWidth:960, clientHeight:640, scale:1 });
  browser.protectedValues.add("synthetic-protected-value");
  const redactedImage = await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration, viewport, kind: "image" });
  assert.equal(redactedImage.ok, true, redactedImage.error?.code);
  assert.equal(redactedImage.result.redaction, "full_viewport");
  const redactedBytes = Buffer.from(redactedImage.result.data_base64, "base64");
  assert.notDeepEqual(redactedBytes, nativeImage);
  assert.equal(redactedBytes.readUInt32BE(16), viewport.css_width);
  const protectedDownload = await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration, viewport, kind: "download", guid: "unobserved" });
  assert.equal(protectedDownload.ok, false);
  assert.equal(protectedDownload.error.code, "browser_observation_redacted");
  browser.protectedValues.clear();
  assert.equal((await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration + 1, viewport, kind: "image" })).ok, false);
  const partial = await request("browser.artifact", { ...current, browser_generation: browser.browserGeneration, viewport, kind: "download", guid: started.data.guid });
  assert.equal(partial.ok, false, "canceled download has no reusable artifact");
  // First-party fail-capable assertion for P8. On frozen G2 this is RED at
  // browser_upload_invalid: a visible trigger is not yet a permitted chooser.
  const chosen = await request("browser.upload", { ...current, node_ref: chooser.node_ref,
    artifact_files: expected.map(file => ({ display_name: file.name, mime_type: file.type,
      data_base64: file.bytes.toString("base64"), size_bytes: file.bytes.length, sha256: createHash("sha256").update(file.bytes).digest("hex") })) });
  assert.equal(chosen.ok, true, chosen.error?.code);
  assert.equal((await request("browser.action", { ...current, node_ref: send.node_ref, action: { kind: "click" } })).ok, true);
  await until(async () => fixture.receipts[1]);
  assertControllerfilesReceipt(fixture.receipts[1], expected);
  const result = { mp: ["MP-08", "MP-10", "MP-11"], scope: "controller/Chromium fixture only",
    inputUpload: true, missingDenied: true, cancelNoPartialReuse: true, downloadExactBytes: true,
    downloadCancelNoPartialReuse: true,
    capture: { sha256: createHash("sha256").update(image).digest("hex"), sizeBytes: image.length,
      targetId: current.target_id, documentId: before, viewport }, passiveEventsRedacted: true, controllerImageArtifact: true, controllerDownloadArtifact: true, controllerPassiveHar: true,
    chooser: { ok: chosen.ok, diagnostic: chosen.error?.code ?? null },
    openGates: ["kernel artifact admission", "kernel browser passive attachment", "provider/Web/TUI conjunction"] };
  if (evidence) {
    await writeFile(path.join(evidence, "controller-result.json"), JSON.stringify(result, null, 2), { mode: 0o600, flag: "wx" });
    await writeFile(path.join(evidence, "same-tab.png"), image, { mode: 0o600, flag: "wx" });
    await writeFile(path.join(evidence, "passive-events.json"), JSON.stringify(events, null, 2), { mode: 0o600, flag: "wx" });
  }
  assert.equal(chosen.ok, true, `visible chooser seam: ${chosen.error?.code}`);
});

async function until(check) {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  throw new Error("bounded fixture condition did not settle");
}
