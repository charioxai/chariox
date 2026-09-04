import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { handleBrowserControllerRequest } from "./browser-controller.mjs";

assert.ok(process.env.PLAYWRIGHT_MODULE, "set PLAYWRIGHT_MODULE to the installed module; this test never downloads a browser");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE);
const viewport = {
  css_width: 1280, css_height: 800, device_scale_factor: 1,
  desktop_pixel_width: 1280, desktop_pixel_height: 800,
};

for (const layout of ["page", "nested-frame", "shadow-root"]) {
test(`replaced ${layout} fields reject their old reference and can be rediscovered`, async () => {
  await withController(async ({ page, request }) => {
    const field = await fixtureField(page, layout);
    const reconciled = await request("browser.reconcile", { viewport });
    assert.equal(reconciled.ok, true);
    const { target_id, document_id } = reconciled.result.tabs[0];
    const target = { target_id, document_id };
    const first = await request("browser.snapshot", target);
    const oldRef = fieldReference(first);
    await field.evaluate((input) => input.replaceWith(input.cloneNode()));

    const rejected = await request("browser.action", {
      ...target, node_ref: oldRef, action: { kind: "fill", text: "must not land" }, timeout_ms: 300,
    });
    assert.equal(rejected.ok, false);
    assert.equal(rejected.error.code, "stale_element_reference");
    assert.equal(await field.inputValue(), "");

    const fresh = await request("browser.snapshot", target);
    const newRef = fieldReference(fresh);
    assert.notEqual(newRef, oldRef);
    const filled = await request("browser.action", {
      ...target, node_ref: newRef, action: { kind: "fill", text: "rediscovered" },
    });
    assert.equal(filled.ok, true, JSON.stringify(filled.error));
    assert.equal(await field.inputValue(), "rediscovered");
  });
});
}

async function fixtureField(page, layout) {
  await page.goto("data:text/html,<main></main>");
  await page.evaluate((layout) => {
    const markup = '<label>Sample<input></label>';
    if (layout === "page") document.querySelector("main").innerHTML = markup;
    if (layout === "shadow-root") document.querySelector("main").attachShadow({ mode: "open" }).innerHTML = markup;
    if (layout === "nested-frame") {
      const inner = document.createElement("iframe");
      inner.srcdoc = markup;
      inner.style = "width:600px;height:100px";
      const outer = document.createElement("iframe");
      outer.srcdoc = inner.outerHTML;
      outer.style = "width:700px;height:200px";
      document.body.append(outer);
    }
  }, layout);
  const field = layout === "nested-frame"
    ? page.frameLocator("iframe").frameLocator("iframe").getByLabel("Sample")
    : page.getByLabel("Sample");
  await field.waitFor();
  return field;
}

test("navigation invalidates the old document while preserving the target for rediscovery", async () => {
  await withController(async ({ page, request }) => {
    await fixtureField(page, "page");
    const original = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const oldRef = fieldReference(await request("browser.snapshot", original));
    await page.goto(`data:text/html,${encodeURIComponent('<title>Replacement</title><label>Sample<input></label>')}`);
    const rejected = await request("browser.action", {
      ...original, node_ref: oldRef, action: { kind: "fill", text: "must not land" },
    });
    assert.equal(rejected.ok, false);
    assert.equal(rejected.error.code, "stale_document_reference");
    assert.equal(await page.getByLabel("Sample").inputValue(), "");
    const current = (await request("browser.reconcile", { viewport })).result.tabs[0];
    assert.equal(current.target_id, original.target_id);
    assert.notEqual(current.document_id, original.document_id);
    const newRef = fieldReference(await request("browser.snapshot", current));
    const filled = await request("browser.action", {
      ...current, node_ref: newRef, action: { kind: "fill", text: "new document" },
    });
    assert.equal(filled.ok, true, JSON.stringify(filled.error));
    assert.equal(await page.getByLabel("Sample").inputValue(), "new document");
  });
});

test("child-frame navigation invalidates old fields without changing the top document", async () => {
  await withController(async ({ page, request }) => {
    const field = await fixtureField(page, "nested-frame");
    const original = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const oldRef = fieldReference(await request("browser.snapshot", original));
    await page.frameLocator("iframe").locator("iframe").evaluate((frame) => {
      frame.srcdoc = '<label>Sample<input data-revision="new"></label>';
    });
    await page.frameLocator("iframe").frameLocator("iframe").locator('[data-revision="new"]').waitFor();
    const rejected = await request("browser.action", {
      ...original, node_ref: oldRef, action: { kind: "fill", text: "must not land" }, timeout_ms: 300,
    });
    assert.equal(rejected.ok, false);
    assert.equal(rejected.error.code, "stale_element_reference");
    assert.equal(await field.inputValue(), "");
    const current = (await request("browser.reconcile", { viewport })).result.tabs[0];
    assert.equal(current.target_id, original.target_id);
    assert.equal(current.document_id, original.document_id);
    const newRef = fieldReference(await request("browser.snapshot", current));
    const filled = await request("browser.action", {
      ...current, node_ref: newRef, action: { kind: "fill", text: "new frame" },
    });
    assert.equal(filled.ok, true, JSON.stringify(filled.error));
    assert.equal(await field.inputValue(), "new frame");
  });
});

function fieldReference(response) {
  assert.equal(response.ok, true, JSON.stringify(response.error));
  const fields = response.result.accessibility_nodes.filter((node) => node.role === "textbox" && node.name.trim() === "Sample");
  assert.equal(fields.length, 1);
  return fields[0].node_ref;
}

async function withController(run) {
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-browser-"));
  let context;
  let browser;
  try {
    context = await chromium.launchPersistentContext(profile, {
      channel: "chrome", headless: true, args: ["--remote-debugging-port=0"],
    });
    const port = Number((await readFile(path.join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]);
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535);
    browser = new BrowserCdpClient({ debuggerEndpoint: `http://127.0.0.1:${port}` });
    const page = context.pages()[0] ?? await context.newPage();
    page.setDefaultTimeout(10_000);
    let nextId = 0;
    const request = (method, params) => handleBrowserControllerRequest({ id: ++nextId, method, params }, { browser });
    await run({ page, request });
  } finally {
    try {
      await browser?.close();
    } finally {
      try {
        await context?.close();
      } finally {
        await rm(profile, { recursive: true, force: true });
      }
    }
  }
}
