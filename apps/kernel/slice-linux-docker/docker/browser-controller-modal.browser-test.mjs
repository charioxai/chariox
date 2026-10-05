// MP-08/MP-10/MP-11: modal navigation must preserve the normal dialog authority.
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { handleBrowserControllerRequest } from "./browser-controller.mjs";
import { browserControllerLaunchOptions } from "./browser-controller-test-launch-options.mjs";

assert.ok(process.env.PLAYWRIGHT_MODULE, "use the installed browser harness; never download");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE);
const viewport = { css_width: 1280, css_height: 800, device_scale_factor: 1,
  desktop_pixel_width: 1280, desktop_pixel_height: 800 };

for (const race of [false, true]) test(`MP-08/MP-10/MP-11 ${race ? "dialog opening during read" : "pending dialog"} reconciles without replying or replaying`, async () => {
  let navigations = 0;
  const server = createServer((request, response) => {
    if (request.url === "/modal") navigations++;
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end('<body><button>Ready</button>' + (race ? '' : '<script>alert("Continue?");document.body.dataset.resolved="yes"</script>') + '</body>');
  });
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-modal-browser-"));
  let context, browser;
  try {
    await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
    context = await chromium.launchPersistentContext(profile, browserControllerLaunchOptions(process.env));
    const page = context.pages()[0];
    page.on("dialog", () => {}); // Only the explicit controller answer may resolve it.
    const port = Number((await readFile(path.join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]);
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535);
    browser = new BrowserCdpClient({ debuggerEndpoint: `http://127.0.0.1:${port}`, requestTimeoutMs: 1000 });
    let id = 0;
    const request = (method, params) => handleBrowserControllerRequest({ id: ++id, method, params }, { browser });
    const initial = (await request("browser.reconcile", { viewport })).result;
    const old = initial.tabs[0];
    const opened = page.waitForEvent("dialog");
    const navigation = await request("browser.navigate", { ...old, url: `http://127.0.0.1:${server.address().port}/modal` });
    assert.equal(navigation.ok, true, JSON.stringify(navigation.error));
    let blockedReads = 0;
    if (race) {
      const connection = await browser.ensureConnection();
      const send = connection.send.bind(connection);
      let armed = true;
      connection.send = async (method, params, sessionId) => {
        if (armed && method === "Page.getFrameTree") {
          armed = false;
          blockedReads++;
          // The real renderer pauses after the observation precheck. This
          // fixture alone opens it; only the normal dialog action answers it.
          void send("Runtime.evaluate", {
            expression: 'alert("Continue?");document.body.dataset.resolved="yes"',
          }, sessionId).catch(() => {});
          await opened;
          const deadline = Date.now() + 1000;
          while (!browser.dialogDefaults.isOpen(old.target_id)) {
            assert.ok(Date.now() < deadline, "controller must observe the dialog");
            await new Promise(resolve => setTimeout(resolve, 5));
          }
        }
        return send(method, params, sessionId);
      };
    } else await opened;
    const reconciled = await request("browser.reconcile", { viewport });
    assert.equal(reconciled.ok, true, JSON.stringify(reconciled.error));
    const target = reconciled.result.tabs[0];
    assert.equal(target.document_id, navigation.result.document_id);
    assert.equal(reconciled.result.focused_target_id, initial.focused_target_id);
    assert.equal(navigations, 1);
    assert.equal(blockedReads, race ? 1 : 0);
    assert.equal(browser.dialogDefaults.isOpen(target.target_id), true);
    const resized = await request("browser.reconcile", { viewport: { ...viewport, css_width: 1024 } });
    assert.equal(resized.error?.code, "browser_dialog_open");
    assert.equal(browser.dialogDefaults.isOpen(target.target_id), true);
    const unchanged = await request("browser.reconcile", { viewport });
    assert.equal(unchanged.ok, true, JSON.stringify(unchanged.error));
    assert.equal(unchanged.result.tabs[0].document_id, target.document_id);
    const stale = await request("browser.snapshot", old);
    assert.equal(stale.error?.code, "stale_document_reference");
    const blocked = await request("browser.snapshot", target);
    assert.equal(blocked.error?.code, "browser_dialog_open");
    assert.equal(browser.dialogDefaults.isOpen(target.target_id), true);
    const handled = await request("browser.dialog", { ...target, action: "accept" });
    assert.equal(handled.ok, true, JSON.stringify(handled.error));
    const snapshot = await request("browser.snapshot", target);
    assert.equal(snapshot.ok, true, JSON.stringify(snapshot.error));
    assert.equal(await page.locator("body").getAttribute("data-resolved"), "yes");
    assert.equal(navigations, 1);
  } finally {
    try { await browser?.close(); }
    finally {
      try { await context?.close(); }
      finally {
        server.closeAllConnections();
        await new Promise(resolve => server.close(resolve));
        await rm(profile, { recursive: true, force: true });
      }
    }
  }
});
