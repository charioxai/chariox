// MP-11 (owner 2026-10-08): real Chromium, real cross-site (isolated) frames.
// Consent/captcha frames stay visible and clickable; only protected fields,
// Vault echoes and marked frames are masked. No mocked DOM or Input dispatch.
import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { KernelBrowserHost } from "./kernel-browser-host.mjs";
import { decodePng } from "./kernel-browser-pixels.mjs";

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "MP-11: explicit disposable drill root required");
const secret = "vault-fixture-7Qx2";
// Top document at 127.0.0.1, frames at localhost: a different site, so
// Chromium renders them in isolated (out-of-process) frames.
const frames = {
  consent: `<body style="margin:0;background:rgb(0,90,200)"><button id="accept" style="position:absolute;left:20px;top:20px;width:200px;height:60px" onclick="document.body.style.background='rgb(0,200,90)'">Accept</button></body>`,
  captcha: `<body style="margin:0;background:rgb(250,250,250)"><div id="box" style="position:absolute;left:10px;top:10px;width:30px;height:30px;background:rgb(200,0,0)" onclick="this.style.background='rgb(0,160,0)'"></div></body>`,
  echo: `<body style="margin:0;background:rgb(255,255,0)"><p style="position:absolute;left:10px;top:100px;margin:0;font:20px monospace">${secret}</p></body>`,
  field: `<body style="margin:0;background:rgb(255,255,0)"><input type="password" value="x" style="position:absolute;left:40px;top:50px;width:100px;height:30px"></body>`,
  plain: `<body style="margin:0;background:rgb(255,255,0)"></body>`,
};
const pages = {
  consent: `<iframe id="c" src="FRAME/consent" style="position:fixed;left:200px;top:100px;width:600px;height:400px;border:0"></iframe>
    <iframe id="r" src="FRAME/captcha" style="position:fixed;left:900px;top:100px;width:304px;height:78px;border:0"></iframe>`,
  echo: `<iframe src="FRAME/echo" style="position:fixed;left:100px;top:100px;width:400px;height:300px;border:0"></iframe>`,
  field: `<iframe src="FRAME/field" style="position:fixed;left:100px;top:100px;width:400px;height:300px;border:0"></iframe>`,
  marked: `<iframe data-chariox-observation-protected src="FRAME/plain" style="position:fixed;left:100px;top:100px;width:300px;height:200px;border:0"></iframe>
    <div data-chariox-secret style="position:fixed;left:600px;top:100px;width:10px;height:10px"><iframe src="FRAME/plain" style="position:absolute;left:0;top:200px;width:300px;height:200px;border:0"></iframe></div>`,
};

async function fixture(name, run) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, "frames-"));
  const server = createServer((request, response) => {
    response.setHeader("Content-Type", "text/html");
    const frame = request.url.startsWith("/frame/") ? frames[request.url.slice(7)] : null;
    response.end(`<!doctype html>${frame ?? `<body style="margin:0;background:rgb(255,255,255)">${pages[name].replaceAll("FRAME", `http://localhost:${server.address().port}/frame`)}</body>`}`);
  });
  const host = new KernelBrowserHost(root);
  try {
    await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
    const opened = await host.request({ op: "open", url: `http://127.0.0.1:${server.address().port}/` });
    const tab = host.tabs.get(opened.tabs[0].tab_id);
    const { connection } = await host.browser.resolvePageTarget(tab.target_id);
    // Wait for every isolated frame to load and be auto-attached.
    const isolated = async () => {
      const { targetInfos } = await connection.send("Target.getTargets");
      return targetInfos.filter(target => target.type === "iframe" && target.url.includes("/frame/"));
    };
    const expected = (pages[name].match(/<iframe/g) ?? []).length;
    for (let tries = 0; tries < 200 && (await isolated()).length < expected; tries++) await new Promise(r => setTimeout(r, 25));
    await new Promise(r => setTimeout(r, 300));
    const state = await host.request({ op: "state" });
    Object.assign(tab, { document_id: state.tabs.find(t => t.tab_id === tab.tab_id).document_id });
    const pixels = async () => { const frame = await host.displayScreenshot(tab); return decodePng(frame.data_base64); };
    const at = (image, x, y) => [...image.pixels.subarray((y * image.width + x) * 4, (y * image.width + x) * 4 + 3)];
    const frameEval = async (urlPart, expression) => {
      const target = (await isolated()).find(t => t.url.includes(urlPart));
      const { sessionId } = await connection.send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
      try { return (await connection.send("Runtime.evaluate", { expression, returnByValue: true }, sessionId)).result.value; }
      finally { await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {}); }
    };
    await run({ host, tab, opened, pixels, at, frameEval });
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
}
const black = [0, 0, 0];

test("MP-11 consent and captcha frames are visible and clickable", async () => {
  await fixture("consent", async ({ host, tab, opened, pixels, at, frameEval }) => {
    let image = await pixels();
    assert.deepEqual(at(image, 500, 450), [0, 90, 200], "consent frame background is visible");
    assert.deepEqual(at(image, 925, 125), [200, 0, 0], "captcha checkbox is visible");
    const input = event => host.request({ op: "input", tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id, input: event });
    await input({ kind: "click", x: 300, y: 150 });
    await input({ kind: "click", x: 925, y: 125 });
    for (let tries = 0; tries < 100 && await frameEval("consent", "document.body.style.background") !== "rgb(0, 200, 90)"; tries++) await new Promise(r => setTimeout(r, 20));
    assert.equal(await frameEval("consent", "document.body.style.background"), "rgb(0, 200, 90)", "the consent click reached the isolated frame");
    assert.equal(await frameEval("captcha", "document.querySelector('#box').style.background"), "rgb(0, 160, 0)");
    await new Promise(r => setTimeout(r, 200));
    image = await pixels();
    assert.deepEqual(at(image, 500, 450), [0, 200, 90], "the clicked consent state is visible");
  });
});

test("MP-11 a protected field inside an isolated frame is masked at its offset only", async () => {
  await fixture("field", async ({ pixels, at }) => {
    const image = await pixels();
    assert.deepEqual(at(image, 190, 165), black, "the password field is masked");
    assert.deepEqual(at(image, 450, 350), [255, 255, 0], "the rest of the frame is visible");
  });
});

test("MP-11 a Vault value echoed by an isolated frame is masked; the frame stays visible", async () => {
  await fixture("echo", async ({ host, pixels, at }) => {
    await host.protect({ values: [secret], targets: [], unknown: false });
    const image = await pixels();
    assert.deepEqual(at(image, 125, 210), black, "the echoed value is masked");
    assert.deepEqual(at(image, 450, 350), [255, 255, 0], "the rest of the frame is visible");
  });
});

test("MP-11 protection markers on a frame owner or its ancestor mask the whole frame", async () => {
  await fixture("marked", async ({ pixels, at }) => {
    const image = await pixels();
    assert.deepEqual(at(image, 380, 280), black, "marked owner");
    assert.deepEqual(at(image, 880, 480), black, "marked ancestor");
    assert.deepEqual(at(image, 1200, 700), [255, 255, 255], "unmarked page");
  });
});
