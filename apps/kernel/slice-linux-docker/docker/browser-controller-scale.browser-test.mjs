// MP-08/MP-10: real Chromium regression for the Room display scale change.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { connectCdpPipe } from "./kernel-browser-cdp-pipe.mjs";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

const viewportAtScale2 = { css_width: 1280, css_height: 720, device_scale_factor: 2, desktop_pixel_width: 2560, desktop_pixel_height: 1440 };

test("a scale change never retires a page that holds unsaved in-memory work", { timeout: 60000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "mirror-scale-"));
  const server = createServer((_request, response) => { response.setHeader("content-type", "text/html"); response.end("<textarea id=draft></textarea>"); });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const child = spawn(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, [`--user-data-dir=${root}/chrome`, "--remote-debugging-pipe", "--headless=new", "--no-sandbox", "--force-device-scale-factor=1", "about:blank"], { stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"] });
  const connection = connectCdpPipe(child.stdio[3], child.stdio[4], 10000);
  const steps = [];
  const browser = new BrowserCdpClient({
    connectionFactory: () => connection,
    applyCanonicalDisplay: async () => { steps.push("display"); },
    applyBrowserScale: async () => { steps.push("relaunch"); await connection.send("Browser.close"); },
    browserScaleChange: async () => "relaunch",
  });
  try {
    const { targetId } = await connection.send("Target.createTarget", { url: `http://127.0.0.1:${server.address().port}/` });
    const { sessionId } = await connection.send("Target.attachToTarget", { targetId, flatten: true });
    const evaluate = async (expression) => (await connection.send("Runtime.evaluate", { expression, returnByValue: true }, sessionId)).result.value;
    for (let attempt = 0; attempt < 100 && await evaluate("document.readyState") !== "complete"; attempt += 1) await new Promise((resolve) => setTimeout(resolve, 50));
    const marker = await evaluate("document.getElementById('draft').value = 'unsaved'; window.documentMarker = Math.random(); window.documentMarker");
    await assert.rejects(browser.reconcile(viewportAtScale2), (error) => error.code === "viewport_apply_failed");
    assert.deepEqual(steps, [], "neither the display nor the browser lifetime was touched");
    assert.equal(await evaluate("document.getElementById('draft').value"), "unsaved");
    assert.equal(await evaluate("window.documentMarker"), marker, "the same document is still live");
  } finally {
    await connection.send("Browser.close").catch(() => {});
    for (let attempt = 0; attempt < 50 && child.exitCode === null && child.signalCode === null; attempt += 1) await new Promise((resolve) => setTimeout(resolve, 100));
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await new Promise((resolve) => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});

async function withBlankBrowser(run, { scaleChange = "relaunch" } = {}) {
  const root = await mkdtemp(path.join(tmpdir(), "mirror-scale-blank-"));
  const child = spawn(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, [`--user-data-dir=${root}/chrome`, "--remote-debugging-pipe", "--headless=new", "--no-sandbox", "--force-device-scale-factor=1", "about:blank"], { stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"] });
  const connection = connectCdpPipe(child.stdio[3], child.stdio[4], 10000);
  const steps = [];
  const browser = new BrowserCdpClient({
    connectionFactory: () => connection,
    applyCanonicalDisplay: async () => { steps.push("display"); },
    applyBrowserScale: async () => { steps.push("relaunch"); await connection.send("Browser.close"); },
    browserScaleChange: async () => scaleChange,
    markBrowserHandedOut: async () => { steps.push("handed out"); },
  });
  try {
    const { targetInfos } = await connection.send("Target.getTargets");
    const { targetId } = targetInfos.find((target) => target.type === "page");
    const { sessionId } = await connection.send("Target.attachToTarget", { targetId, flatten: true });
    const evaluate = async (expression) => (await connection.send("Runtime.evaluate", { expression, returnByValue: true }, sessionId)).result.value;
    await run({ browser, steps, evaluate });
  } finally {
    await connection.send("Browser.close").catch(() => {});
    for (let attempt = 0; attempt < 50 && child.exitCode === null && child.signalCode === null; attempt += 1) await new Promise((resolve) => setTimeout(resolve, 100));
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await rm(root, { recursive: true, force: true });
  }
}

// A shadow-tree editor leaves the light DOM an empty skeleton and adds no
// window property; nothing in the document tells it apart from a fresh page.
const SHADOW_EDITOR = "document.body.attachShadow({ mode: 'open' }).innerHTML = '<textarea>unsaved draft</textarea>'";
const SHADOW_DRAFT = "document.body.shadowRoot?.querySelector('textarea')?.value";

test("a scale change keeps an about:blank startup page alive once it was handed out", { timeout: 60000 }, async () => {
  await withBlankBrowser(async ({ browser, steps, evaluate }) => {
    await browser.handOutBrowser();
    await evaluate(SHADOW_EDITOR);
    await assert.rejects(browser.reconcile(viewportAtScale2), (error) => error.code === "viewport_apply_failed");
    assert.deepEqual(steps, ["handed out"], "neither the display nor the browser lifetime was touched");
    assert.equal(await evaluate(SHADOW_DRAFT), "unsaved draft", "the same document and its draft are still live");
  });
});

test("a scale change keeps a startup page alive that the supervisor records as handed out", { timeout: 60000 }, async () => {
  // A restarted controller has handed nothing out itself; the lifetime record does.
  await withBlankBrowser(async ({ browser, steps, evaluate }) => {
    await evaluate(SHADOW_EDITOR);
    await assert.rejects(browser.reconcile(viewportAtScale2), (error) => error.code === "viewport_apply_failed");
    assert.deepEqual(steps, []);
    assert.equal(await evaluate(SHADOW_DRAFT), "unsaved draft");
  }, { scaleChange: "handed_out" });
});

test("a scale change relaunches a startup lifetime nobody was handed", { timeout: 60000 }, async () => {
  await withBlankBrowser(async ({ browser, steps }) => {
    await browser.reconcile(viewportAtScale2);
    assert.deepEqual(steps, ["display", "relaunch"]);
  });
});
