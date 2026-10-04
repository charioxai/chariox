import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, stat, statfs, symlink, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { startBrowserComputerFixture } from "../../../cli/scripts/lib/browser-computer-fixture.mjs";
import { browserControllerLaunchOptions } from "./browser-controller-test-launch-options.mjs";
import { BrowserCdpClient, BrowserControllerError } from "./browser-controller-cdp.mjs";
import { handleBrowserControllerRequest } from "./browser-controller.mjs";

const browserLaunchOptions = browserControllerLaunchOptions(process.env);
assert.ok(process.env.PLAYWRIGHT_MODULE, "set PLAYWRIGHT_MODULE to the installed module; this test never downloads a browser");
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE);

const viewport = {
  css_width: 1280, css_height: 800, device_scale_factor: 1,
  desktop_pixel_width: 1280, desktop_pixel_height: 800,
};

test("MP-08/MP-10/MP-11 visible native labels ground styled check/radio controls", async () => {
  await withController(async ({page, request}) => {
    await page.setContent(`<style>input {display:none} label {display:block;margin:16px;padding:12px}</style>
      <input id="agree" type="checkbox"><label for="agree" id="agree-label">Agreement</label>
      <input id="first" type="radio" name="choice"><label for="first" id="first-label">First choice</label>
      <input id="second" type="radio" name="choice"><label for="second" id="second-label">Second choice</label>
      <input id="disabled" type="checkbox" disabled><label for="disabled" id="disabled-label">Disabled choice</label>
      <label for="missing" id="inert-label">Inert label</label><output id="events">0</output>
      <script>document.addEventListener('change', () => {events.textContent=Number(events.textContent)+1})</script>`);
    const target = (await request("browser.reconcile", {viewport})).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    const label = id => snapshot.dom_nodes.find(n => n.attributes.id === id);
    for (const id of ["agree-label", "first-label", "second-label"]) {
      assert.equal(label(id).clickable, true, `MP-08/MP-10/MP-11 ${id} must expose native activation evidence`);
      assert.equal(label(id).rendered, true);
      const clicked = await request("browser.action", {...target, node_ref: label(id).node_ref, action: {kind: "click"}});
      assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
    }
    assert.equal(await page.locator("#agree").isChecked(), true);
    assert.equal(await page.locator("#first").isChecked(), false);
    assert.equal(await page.locator("#second").isChecked(), true);
    assert.equal(await page.locator("#events").textContent(), "3");
    assert.equal(label("disabled-label").clickable, false);
    assert.equal(label("inert-label").clickable, false);
    // A control disabled after discovery must still reject before native input.
    await page.locator("#agree").evaluate(input => {input.disabled = true});
    const disabled = await request("browser.action", {...target, node_ref: label("agree-label").node_ref,
      action: {kind: "click"}, timeout_ms: 100});
    assert.equal(disabled.error?.code, "browser_element_disabled");
    assert.equal(await page.locator("#events").textContent(), "3");
  });
});

test("MP-08/MP-10/MP-11 offscreen and covered targets distinguish actionability from CDP loss", async () => {
  await withController(async ({page, request}) => {
    await page.setContent(`<button id="covered" onclick="this.dataset.clicks=Number(this.dataset.clicks||0)+1">Covered</button>
      <div style="position:fixed;inset:0;background:white;z-index:1" id="cover">Overlay</div>
      <button style="margin-top:1800px;display:block" id="offscreen" onclick="this.dataset.clicks=Number(this.dataset.clicks||0)+1">Offscreen</button>`);
    const target = (await request("browser.reconcile", {viewport})).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    const reference = id => snapshot.dom_nodes.find(n => n.attributes.id === id).node_ref;
    const blocked = await request("browser.action", {...target, node_ref: reference("covered"), action: {kind: "click"}, timeout_ms: 100});
    assert.equal(blocked.error?.code, "browser_element_obscured");
    assert.ok(blocked.error.message.includes("no input was delivered"));
    assert.equal(await page.locator("#covered").getAttribute("data-clicks"), null);
    await page.locator("#cover").evaluate(cover => cover.remove());
    const clicked = await request("browser.action", {...target, node_ref: reference("offscreen"), action: {kind: "click"}});
    assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
    assert.equal(await page.locator("#offscreen").getAttribute("data-clicks"), "1");
    assert.equal((await request("browser.snapshot", target)).ok, true);
  });
});

test("MP-08/MP-10/MP-11 an input acknowledgement failure never replays a delivered click", async () => {
  await withController(async ({page, request, browser}) => {
    await page.setContent('<button onclick="this.dataset.clicks=Number(this.dataset.clicks||0)+1">Count click</button>');
    const target = (await request("browser.reconcile", {viewport})).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    const button = snapshot.accessibility_nodes.find(n => n.role === "button" && n.name === "Count click");
    const connection = browser.connection;
    const send = connection.send.bind(connection);
    const calls = [];
    let lostAck = false;
    connection.send = async (method, params, sessionId) => {
      calls.push({method, params});
      const result = await send(method, params, sessionId);
      if (!lostAck && method === "Input.dispatchMouseEvent" && params.type === "mouseReleased") {
        lostAck = true;
        throw new BrowserControllerError("browser_cdp_timeout", "fixture lost acknowledgement after native input");
      }
      return result;
    };
    const failed = await request("browser.action", {...target, node_ref: button.node_ref, action: {kind: "click"}});
    assert.equal(failed.error?.code, "browser_cdp_timeout");
    assert.equal(await page.locator("button").getAttribute("data-clicks"), "1");
    assert.equal(calls.filter(c => c.method === "Input.dispatchMouseEvent" && c.params.type === "mousePressed").length, 1);
    assert.ok(!calls.some(c => c.method === "Target.detachFromTarget"));
    assert.equal((await request("browser.snapshot", target)).ok, true);
    assert.equal(await page.locator("button").getAttribute("data-clicks"), "1");
  });
});

test("test Chromium override is trimmed, absolute-only, and keeps the chrome default", () => {
  assert.deepEqual(
    browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM: "  /opt/chromium  " }),
    {
      executablePath: "/opt/chromium",
      headless: true,
      args: ["--remote-debugging-port=0", "--site-per-process"],
    },
  );
  assert.deepEqual(browserControllerLaunchOptions({}), {
    channel: "chrome",
    headless: true,
    args: ["--remote-debugging-port=0", "--site-per-process"],
  });
  assert.deepEqual(browserControllerLaunchOptions({
    CHARIOX_TEST_CHROMIUM: "/usr/bin/chromium",
    CHARIOX_TEST_CHROMIUM_MODE: "headed",
    DISPLAY: ":99",
  }), {
    executablePath: "/usr/bin/chromium",
    headless: false,
    args: ["--remote-debugging-port=0", "--site-per-process"],
  });
  assert.throws(
    () => browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM: "relative/chromium" }),
    /absolute executable path/,
  );
  assert.throws(
    () => browserControllerLaunchOptions({ CHARIOX_TEST_CHROMIUM_MODE: "headed" }),
    /headed mode requires DISPLAY/,
  );
});

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

test("popup tabs are discovered, activated, and closed through stable controller operations", async () => {
  await withController(async ({ page, request }) => {
    await page.setContent('<a target="_blank" href="about:blank">Open popup</a>');
    const original = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = await request("browser.snapshot", original);
    const link = snapshot.result.accessibility_nodes.find(
      (node) => node.role === "link" && node.name === "Open popup",
    );
    assert.ok(link);

    const popupOpened = page.waitForEvent("popup");
    const clicked = await request("browser.action", {
      ...original,
      node_ref: link.node_ref,
      action: { kind: "click" },
    });
    assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
    const popupPage = await popupOpened;
    await popupPage.waitForLoadState();

    const withPopup = (await request("browser.reconcile", { viewport })).result;
    assert.equal(withPopup.tabs.length, 2);
    const popup = withPopup.tabs.find((tab) => tab.target_id !== original.target_id);
    assert.ok(popup);
    const activated = await request("browser.tab", { ...popup, action: "activate" });
    assert.equal(activated.ok, true, JSON.stringify(activated.error));
    const active = (await request("browser.reconcile", { viewport })).result;
    assert.equal(active.focused_target_id, popup.target_id);

    const closed = await request("browser.tab", { ...popup, action: "close" });
    assert.equal(closed.ok, true, JSON.stringify(closed.error));
    const afterCloseResponse = await request("browser.reconcile", { viewport });
    assert.equal(afterCloseResponse.ok, true, JSON.stringify(afterCloseResponse.error));
    const afterClose = afterCloseResponse.result;
    assert.deepEqual(afterClose.tabs.map((tab) => tab.target_id), [original.target_id]);
    assert.equal(afterClose.focused_target_id, original.target_id);
  });
});

test("OAuth popup redirect and callback preserve stable tabs and authenticate the original page", async () => {
  const fixture = await startBrowserComputerFixture({ password: "fixture-oauth-password" });
  try {
    await withController(async ({ page, request }) => {
      await page.goto(`${fixture.origin}/oauth/start`);
      const original = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const startSnapshot = await request("browser.snapshot", original);
      const signIn = startSnapshot.result.accessibility_nodes.find(
        (node) => node.role === "link" && node.name === "Sign in with Fixture",
      );
      assert.ok(signIn);

      const popupOpened = page.waitForEvent("popup");
      const opened = await request("browser.action", {
        ...original,
        node_ref: signIn.node_ref,
        action: { kind: "click" },
      });
      assert.equal(opened.ok, true, JSON.stringify(opened.error));
      const popupPage = await popupOpened;
      await popupPage.waitForURL(/\/oauth\/authorize\?state=/);

      const withPopup = (await request("browser.reconcile", { viewport })).result;
      const popup = withPopup.tabs.find((tab) => tab.target_id !== original.target_id);
      assert.ok(popup);
      assert.match(popup.url, /\/oauth\/authorize\?state=/);
      const activated = await request("browser.tab", { ...popup, action: "activate" });
      assert.equal(activated.ok, true, JSON.stringify(activated.error));

      const authorizeSnapshot = await request("browser.snapshot", popup);
      const authorize = authorizeSnapshot.result.accessibility_nodes.find(
        (node) => node.role === "button" && node.name === "Authorize Fixture account",
      );
      assert.ok(authorize);
      const callbackReached = popupPage.waitForURL(/\/oauth\/callback\?/);
      const authorized = await request("browser.action", {
        ...popup,
        node_ref: authorize.node_ref,
        action: { kind: "click" },
      });
      assert.equal(authorized.ok, true, JSON.stringify(authorized.error));
      await callbackReached;
      await page.waitForFunction(() => document.querySelector("#oauth-status")?.textContent?.includes("CHARIOX_FIXTURE_OAUTH_AUTHENTICATED"));

      const afterCallback = (await request("browser.reconcile", { viewport })).result;
      const callbackTab = afterCallback.tabs.find((tab) => tab.target_id === popup.target_id);
      assert.ok(callbackTab);
      assert.notEqual(callbackTab.document_id, popup.document_id);
      assert.equal(afterCallback.tabs.find((tab) => tab.target_id === original.target_id)?.document_id, original.document_id);
      assert.match(await page.locator("#oauth-status").textContent(), /agent@chariox\.test/);
      assert.match(await page.evaluate(async () => await fetch("/mail/inbox").then((response) => response.text())), /CHARIOX_FIXTURE_INBOX/);

      const callbackSnapshot = await request("browser.snapshot", callbackTab);
      const finish = callbackSnapshot.result.accessibility_nodes.find(
        (node) => node.role === "button" && node.name === "Complete sign-in",
      );
      assert.ok(finish);
      const popupClosed = popupPage.waitForEvent("close");
      const finished = await request("browser.action", {
        ...callbackTab,
        node_ref: finish.node_ref,
        action: { kind: "click" },
      });
      assert.equal(finished.ok, true, JSON.stringify(finished.error));
      await popupClosed;

      const settled = (await request("browser.reconcile", { viewport })).result;
      assert.deepEqual(settled.tabs.map((tab) => tab.target_id), [original.target_id]);
      assert.equal(settled.focused_target_id, original.target_id);
    });
  } finally {
    await fixture.close();
  }
});

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

test("controller dialogs accept, dismiss, preserve prompt values, and recover from absent dialogs", { timeout: 30_000 }, async () => {
  await withController(async ({ page, request }) => {
    // Keep Playwright from auto-dismissing dialogs. Only the controller may answer.
    page.on("dialog", () => {});
    await page.goto(`data:text/html,${encodeURIComponent('<button>Open dialog</button><output>waiting</output><label>Sample<input></label>')}`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    for (const sample of [
      { type: "alert", action: "accept", expected: "undefined" },
      { type: "alert", action: "dismiss", expected: "undefined" },
      { type: "confirm", action: "accept", expected: "true" },
      { type: "confirm", action: "dismiss", expected: "false" },
      { type: "prompt", action: "accept", prompt_text: "Mañana 日本語", expected: "Mañana 日本語" },
      { type: "prompt", action: "accept", prompt_text: "", expected: "" },
      { type: "prompt", action: "accept", expected: "default value" },
      { type: "prompt", action: "accept", default_text: "é".repeat(1025), default_error: true, prompt_text: "override", expected: "override" },
      { type: "prompt", action: "dismiss", prompt_text: "ignored", expected: "null" },
    ]) {
      await page.evaluate(({ type, default_text = "default value" }) => {
        document.querySelector("output").textContent = "waiting";
        document.querySelector("button").onclick = () => {
          document.querySelector("output").textContent = String(window[type]("Sample dialog", default_text));
        };
      }, sample);
      const snapshot = await request("browser.snapshot", target);
      const button = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Open dialog");
      assert.ok(button);
      const clicked = await request("browser.action", { ...target, node_ref: button.node_ref, action: { kind: "click" } });
      assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
      assert.equal(clicked.result.dialog_opened, true);
      const stale = await request("browser.dialog", { ...target, document_id: "previous-document", action: "dismiss" });
      assert.equal(stale.ok, false);
      assert.equal(stale.error.code, "stale_document_reference");
      const invalid = await request("browser.dialog", { ...target, action: "accept", prompt_text: "x".repeat(2049) });
      assert.equal(invalid.ok, false);
      assert.equal(invalid.error.code, "browser_dialog_invalid");
      if (sample.default_error) {
        const oversizedDefault = await request("browser.dialog", { ...target, action: "accept" });
        assert.equal(oversizedDefault.ok, false);
        assert.equal(oversizedDefault.error.code, "browser_dialog_invalid");
      }
      const handled = await request("browser.dialog", { ...target, action: sample.action, prompt_text: sample.prompt_text });
      assert.equal(handled.ok, true, JSON.stringify(handled.error));
      assert.equal(await page.locator("output").textContent(), sample.expected);
      const absent = await request("browser.dialog", { ...target, action: "accept" });
      assert.equal(absent.ok, false);
      assert.equal(absent.error.code, "browser_cdp_command_failed");
      const filled = await request("browser.action", {
        ...target, node_ref: fieldReference(await request("browser.snapshot", target)),
        action: { kind: "fill", text: "after dialog" },
      });
      assert.equal(filled.ok, true, JSON.stringify(filled.error));
      assert.equal(await page.getByLabel("Sample").inputValue(), "after dialog");
    }
    const trace = await request("browser.events.poll", { cursor: 0, browser_generation: 1, limit: 200 });
    assert.equal(trace.ok, true);
    assert.equal(trace.result.replay_gap, false);
    const dialogs = trace.result.events.filter((event) => event.kind.startsWith("dialog_"));
    assert.equal(dialogs.length, 18);
    assert.deepEqual(dialogs.map((event) => event.kind), Array(9).fill(["dialog_opened", "dialog_closed"]).flat());
    assert.ok(dialogs.every((event) => event.target_id === target.target_id && event.document_id === target.document_id));
    assert.ok(dialogs.every((event) => !JSON.stringify(event).includes("Sample dialog") && !JSON.stringify(event).includes("Mañana 日本語") && !JSON.stringify(event).includes("default value")));
  });
});

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

test("history back, forward, and reload preserve the tab and return each new document", async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request }) => {
      await page.goto(`${url}first`);
      await page.goto(`${url}second`);
      await page.goto(`${url}third`);
      const third = (await request("browser.reconcile", { viewport })).result.tabs[0];

      const invalid = await request("browser.history", { ...third, action: "sideways" });
      assert.equal(invalid.ok, false);
      assert.equal(invalid.error.code, "browser_history_action_invalid");

      const back = await request("browser.history", { ...third, action: "back" });
      assert.equal(back.ok, true, JSON.stringify(back.error));
      assert.equal(back.result.target_id, third.target_id);
      assert.notEqual(back.result.document_id, third.document_id);
      assert.equal(back.result.url, `${url}second`);
      assert.equal(page.url(), `${url}second`);

      const stale = await request("browser.history", { ...third, action: "back" });
      assert.equal(stale.ok, false);
      assert.equal(stale.error.code, "stale_document_reference");

      const forward = await request("browser.history", { ...back.result, action: "forward" });
      assert.equal(forward.ok, true, JSON.stringify(forward.error));
      assert.equal(forward.result.target_id, third.target_id);
      assert.equal(forward.result.url, `${url}third`);
      assert.equal(page.url(), `${url}third`);

      const unavailable = await request("browser.history", { ...forward.result, action: "forward" });
      assert.equal(unavailable.ok, false);
      assert.equal(unavailable.error.code, "browser_history_unavailable");

      const reloaded = await request("browser.history", { ...forward.result, action: "reload" });
      assert.equal(reloaded.ok, true, JSON.stringify(reloaded.error));
      assert.equal(reloaded.result.target_id, third.target_id);
      assert.notEqual(reloaded.result.document_id, forward.result.document_id);
      assert.equal(reloaded.result.url, `${url}third`);
      assert.equal(page.url(), `${url}third`);
    });
  });
});

test("same-document history preserves the current document identity", async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request }) => {
      await page.goto(`${url}spa`);
      await page.evaluate(() => {
        history.pushState({ step: 1 }, "", "/spa/one");
        history.pushState({ step: 2 }, "", "/spa/two");
      });
      const second = (await request("browser.reconcile", { viewport })).result.tabs[0];

      const back = await request("browser.history", { ...second, action: "back" });
      assert.equal(back.ok, true, JSON.stringify(back.error));
      assert.equal(back.result.target_id, second.target_id);
      assert.equal(back.result.document_id, second.document_id);
      assert.equal(back.result.url, `${url}spa/one`);

      const forward = await request("browser.history", { ...back.result, action: "forward" });
      assert.equal(forward.ok, true, JSON.stringify(forward.error));
      assert.equal(forward.result.target_id, second.target_id);
      assert.equal(forward.result.document_id, second.document_id);
      assert.equal(forward.result.url, `${url}spa/two`);
    });
  });
});

test("a page-command timeout while a prompt is open does not prevent answering or later input", { timeout: 15_000 }, async () => {
  await withController(async ({ page, request }) => {
    page.on("dialog", () => {});
    await page.goto(`data:text/html,${encodeURIComponent('<button onclick="document.querySelector(\'output\').textContent=prompt(\'Question\',\'kept default\')">Prompt</button><output>waiting</output><label>Sample<input></label>')}`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = await request("browser.snapshot", target);
    const button = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Prompt");
    const clicked = await request("browser.action", { ...target, node_ref: button.node_ref, action: { kind: "click" } });
    assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
    assert.equal(clicked.result.dialog_opened, true);
    const timedOut = await request("browser.snapshot", target);
    assert.equal(timedOut.ok, false);
    assert.equal(timedOut.error.code, "browser_cdp_timeout");
    const answered = await request("browser.dialog", { ...target, action: "accept" });
    assert.equal(answered.ok, true, JSON.stringify(answered.error));
    assert.equal(await page.locator("output").textContent(), "kept default");
    const filled = await request("browser.action", {
      ...target, node_ref: fieldReference(await request("browser.snapshot", target)),
      action: { kind: "fill", text: "after timeout" },
    });
    assert.equal(filled.ok, true, JSON.stringify(filled.error));
    assert.equal(await page.getByLabel("Sample").inputValue(), "after timeout");
  }, { requestTimeoutMs: 500 });
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

for (const nested of [false, true]) {
test(`cross-site ${nested ? "nested " : ""}isolated frame fields are discoverable and editable`, async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request, context }) => {
      await page.goto(url);
      const frame = nested ? page.frameLocator("iframe").frameLocator("iframe") : page.frameLocator("iframe");
      const field = frame.getByLabel("Sample");
      await field.waitFor();
      const session = await context.newCDPSession(page);
      try {
        const { targetInfos } = await session.send("Target.getTargets");
        assert.ok(targetInfos.some((target) => target.type === "iframe" && target.url.includes("localhost")), "fixture must run its child in a separate renderer target");
      } finally {
        await session.detach();
      }
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const snapshot = await request("browser.snapshot", target);
      const ref = fieldReference(snapshot);
      const filled = await request("browser.action", {
        ...target, node_ref: ref, action: { kind: "fill", text: "cross origin" },
      });
      assert.equal(filled.ok, true, JSON.stringify(filled.error));
      assert.equal(await field.inputValue(), "cross origin");
      const button = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Accept");
      assert.ok(button);
      const clicked = await request("browser.action", { ...target, node_ref: button.node_ref, action: { kind: "click" } });
      assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
      assert.equal(await frame.getByRole("status").innerText(), "accepted");
    });
  }, { nested });
});
}

test("isolated-frame navigation rejects old references and preserves the parent tab", async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request }) => {
      await page.goto(url);
      const field = page.frameLocator("iframe").getByLabel("Sample");
      await field.waitFor();
      const original = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const oldRef = fieldReference(await request("browser.snapshot", original));
      await page.frames()[1].goto(`http://localhost:${new URL(url).port}/field?revision=2`);
      await field.waitFor();
      const rejected = await request("browser.action", { ...original, node_ref: oldRef, action: { kind: "fill", text: "must not land" } });
      assert.equal(rejected.ok, false);
      assert.equal(rejected.error.code, "stale_element_reference");
      assert.equal(await field.inputValue(), "");
      const current = (await request("browser.reconcile", { viewport })).result.tabs[0];
      assert.equal(current.target_id, original.target_id);
      assert.equal(current.document_id, original.document_id);
      const newRef = fieldReference(await request("browser.snapshot", current));
      assert.notEqual(newRef, oldRef);
      const filled = await request("browser.action", { ...current, node_ref: newRef, action: { kind: "fill", text: "new isolated document" } });
      assert.equal(filled.ok, true, JSON.stringify(filled.error));
      assert.equal(await field.inputValue(), "new isolated document");
    });
  });
});

// MP-08/MP-10: structure-only password challenge, served locally. No service login.
const passwordStepMarkup = `
  <main><section><div><label>Enter your password
    <input type="password" name="Passwd" autocomplete="current-password">
  </label></div></section>
  <div id="passwordNext"><button type="button">Next</button></div>
  <output role="status"></output></main>
  <script>
    const input = document.querySelector('input');
    const output = document.querySelector('output');
    let edited = false;
    input.addEventListener('input', () => { edited = input.value.length > 0; });
    document.querySelector('button').addEventListener('click', () => {
      output.textContent = edited ? 'accepted' : 'empty';
    });
  </script>`;

for (const layout of ["page", "nested-frame", "shadow-root"]) {
  test(`password step ${layout} fills a discovered opaque field and uses its separate Next button`, async () => {
    await withPasswordStep(async (url) => {
      await withController(async ({ page, request }) => {
        await page.goto(url);
        if (layout === "nested-frame") {
          await page.setContent(`<iframe src="${url}"></iframe>`);
        } else if (layout === "shadow-root") {
          await page.evaluate(() => {
            const main = document.querySelector('main');
            const host = document.createElement('div');
            host.attachShadow({ mode: 'open' }).append(main);
            document.body.append(host);
          });
        }
        const field = layout === "nested-frame"
          ? page.frameLocator('iframe').getByLabel('Enter your password')
          : page.getByLabel('Enter your password');
        await field.waitFor();
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const snapshot = await request("browser.snapshot", target);
        const node = snapshot.result.accessibility_nodes.find((node) => node.role === "textbox" && node.name.trim() === "Enter your password");
        assert.ok(node?.node_ref);
        const result = await request("browser.action", {
          ...target, node_ref: node.node_ref,
          action: { kind: "fill", text: "local-password-canary", expected_document_url: url, submit: false },
        });
        assert.equal(result.ok, true, JSON.stringify(result.error));
        assert.equal(await field.inputValue(), "local-password-canary");
        assert.equal(JSON.stringify(result).includes("local-password-canary"), false);
        const next = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Next");
        const clicked = await request("browser.action", { ...target, node_ref: next.node_ref, action: { kind: "click" } });
        assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
        const scope = layout === "nested-frame" ? page.frameLocator('iframe') : page;
        assert.equal(await scope.getByRole('status').innerText(), "accepted");
      });
    });
  });
}

test("password step without a form rejects submit before inserting a secret with an actionable error", async () => {
  await withPasswordStep(async (url) => {
    await withController(async ({ page, request }) => {
      await page.goto(url);
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const snapshot = await request("browser.snapshot", target);
      const node = snapshot.result.accessibility_nodes.find((node) => node.role === "textbox" && node.name.trim() === "Enter your password");
      const result = await request("browser.action", {
        ...target, node_ref: node.node_ref,
        action: { kind: "fill", text: "local-password-canary", expected_document_url: url, submit: true },
      });
      assert.equal(result.ok, false);
      assert.equal(result.error.code, "browser_submit_failed");
      assert.match(result.error.message, /submit=false.*button/);
      assert.equal(await page.getByLabel('Enter your password').inputValue(), "");
      assert.equal(JSON.stringify(result).includes("local-password-canary"), false);
    });
  });
});

test("password step form controls cannot shadow native submit methods", async () => {
  await withController(async ({ page, request }) => {
    await page.setContent(`<form><label>Sample<input type="password"></label>
      <input name="requestSubmit" type="hidden"><input name="submit" type="hidden">
      <output role="status"></output></form>`);
    await page.locator('form').evaluate((form) => {
      form.addEventListener('submit', (event) => {
        event.preventDefault();
        form.querySelector('output').textContent = 'accepted';
      });
    });
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const result = await request("browser.action", {
      ...target, node_ref: fieldReference(await request("browser.snapshot", target)),
      action: { kind: "fill", text: "local-password-canary", expected_document_url: page.url(), submit: true },
    });
    assert.equal(result.ok, true, JSON.stringify(result.error));
    assert.equal(await page.getByRole('status').innerText(), 'accepted');
    assert.equal(JSON.stringify(result).includes("local-password-canary"), false);
  });
});

for (const failure of ["script-exception", "missing-value-setter"]) {
  test(`password step ${failure} reports its safe failure category`, async () => {
    await withPasswordStep(async (url) => {
      await withController(async ({ page, request }) => {
        await page.goto(url);
        await page.getByLabel('Enter your password').evaluate((input, failure) => {
          if (failure === 'script-exception') {
            input.focus = () => { throw new Error('local-password-canary'); };
          } else {
            const prototype = HTMLInputElement.prototype;
            const getter = Object.getOwnPropertyDescriptor(prototype, 'value').get;
            Object.defineProperty(prototype, 'value', { get: getter, set: undefined, configurable: true });
          }
        }, failure);
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const snapshot = await request("browser.snapshot", target);
        const node = snapshot.result.accessibility_nodes.find((node) => node.role === "textbox" && node.name.trim() === "Enter your password");
        const result = await request("browser.action", {
          ...target, node_ref: node.node_ref,
          action: { kind: "fill", text: "local-password-canary", expected_document_url: url, submit: false },
        });
        assert.equal(result.ok, false);
        if (failure === 'script-exception') {
          assert.equal(result.error.code, 'browser_secret_input_exception');
        } else {
          assert.equal(result.error.code, 'browser_action_failed');
          assert.match(result.error.message, /native value setter/);
        }
        assert.equal(JSON.stringify(result).includes('local-password-canary'), false);
        assert.equal(await page.getByLabel('Enter your password').inputValue(), '');
      });
    });
  });
}

async function withPasswordStep(run) {
  const server = createServer((request, response) => {
    response.writeHead(200, { "content-type": "text/html" });
    response.end(passwordStepMarkup);
  });
  try {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    await run(`http://127.0.0.1:${server.address().port}/v3/signin/challenge/pwd`);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  }
}

for (const transitionEvent of ["input", "change"]) {
  test(`secret fill rejects and remasks when a real ${transitionEvent} handler unmasks the field`, { timeout: 15_000 }, async () => {
    await withController(async ({ page, request }) => {
      await page.setContent('<label>Sample<input type="password"></label>');
      const field = page.getByLabel("Sample");
      await field.evaluate((input, eventType) => {
        input.addEventListener(eventType, () => { input.type = "text"; });
      }, transitionEvent);
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const snapshot = await request("browser.snapshot", target);
      const result = await request("browser.action", {
        ...target,
        node_ref: fieldReference(snapshot),
        action: {
          kind: "fill",
          text: "browser-fixture-secret-canary",
          expected_document_url: page.url(),
        },
      });

      assert.equal(result.ok, false);
      assert.equal(result.error.code, "browser_secret_target_not_masked");
      assert.equal(JSON.stringify(result).includes("browser-fixture-secret-canary"), false);
      assert.equal(await field.getAttribute("type"), "password");
    });
  });
}

test("same-site cross-origin frame actions use top-viewport coordinates", async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request, context }) => {
      await page.goto(url);
      await page.frameLocator("iframe").getByLabel("Sample").waitFor();
      const session = await context.newCDPSession(page);
      try {
        const { targetInfos } = await session.send("Target.getTargets");
        assert.equal(targetInfos.filter((target) => target.type === "iframe").length, 0, "same-site cross-origin fixture must share the parent renderer");
      } finally { await session.detach(); }
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const snapshot = await request("browser.snapshot", target);
      const filled = await request("browser.action", { ...target, node_ref: fieldReference(snapshot), action: { kind: "fill", text: "same site" }, timeout_ms: 300 });
      assert.equal(filled.ok, true, JSON.stringify(filled.error));
      assert.equal(await page.frameLocator("iframe").getByLabel("Sample").inputValue(), "same site");
      const button = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Accept");
      const clicked = await request("browser.action", { ...target, node_ref: button.node_ref, action: { kind: "click" } });
      assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
      assert.equal(await page.frameLocator("iframe").getByRole("status").innerText(), "accepted");
    });
  }, { sameSite: true });
});

for (const layout of ["same-site", "isolated", "nested-isolated"]) {
test(`repeated late ${layout} replacements preserve native controller clicks`, { timeout: 30_000 }, async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request }) => {
      await page.goto(url);
      const frame = layout === "nested-isolated"
        ? page.frameLocator("iframe").frameLocator("iframe") : page.frameLocator("iframe");
      await frame.getByLabel("Sample").waitFor();
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      let snapshot = await request("browser.snapshot", target);
      for (let turn = 0; turn < 12; turn++) {
        assert.equal(snapshot.ok, true, JSON.stringify(snapshot.error));
        const oldButton = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Accept");
        assert.ok(oldButton);
        await page.locator("iframe").evaluate((iframe) => iframe.replaceWith(iframe.cloneNode()));
        await frame.getByLabel("Sample").waitFor();
        const stale = await request("browser.action", {
          ...target, node_ref: oldButton.node_ref, action: { kind: "click" }, timeout_ms: 300,
        });
        assert.equal(stale.ok, false, `turn ${turn}: old frame must reject clicks`);
        assert.equal(stale.error.code, "stale_element_reference");
        assert.equal(await frame.getByRole("status").innerText(), "");
        snapshot = await request("browser.snapshot", target);
        assert.equal(snapshot.ok, true, JSON.stringify(snapshot.error));
        const button = snapshot.result.accessibility_nodes.find((node) => node.role === "button" && node.name === "Accept");
        assert.ok(button);
        const clicked = await request("browser.action", {
          ...target, node_ref: button.node_ref, action: { kind: "click" }, timeout_ms: 1_000,
        });
        assert.equal(clicked.ok, true, `turn ${turn}: ${JSON.stringify(clicked.error)}`);
        assert.equal(await frame.getByRole("status").innerText(), "accepted", `turn ${turn}: the click must reach the replacement frame`);
      }
    });
  }, { sameSite: layout === "same-site", nested: layout === "nested-isolated" });
});
}

test("isolated references cannot address a different tab and keep colliding renderer IDs distinct", async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request, context }) => {
      await page.goto(url);
      await page.frameLocator("iframe").getByLabel("Sample").waitFor();
      const original = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const oldRef = fieldReference(await request("browser.snapshot", original));
      const other = await context.newPage();
      await other.goto(`${url}?other=1`);
      await other.frameLocator("iframe").getByLabel("Sample").waitFor();
      const otherTarget = (await request("browser.reconcile", { viewport })).result.tabs.find((tab) => tab.target_id !== original.target_id);
      const rejected = await request("browser.action", { ...otherTarget, node_ref: oldRef, action: { kind: "fill", text: "wrong tab" } });
      assert.equal(rejected.ok, false);
      assert.equal(rejected.error.code, "stale_element_reference");
      assert.equal(await other.frameLocator("iframe").getByLabel("Sample").inputValue(), "");
      const snapshot = await request("browser.snapshot", otherTarget);
      assert.equal(snapshot.ok, true);
      const refs = snapshot.result.dom_nodes.map((node) => node.node_ref);
      assert.equal(new Set(refs).size, refs.length);
      const frameDocument = snapshot.result.dom_documents.find((document) => document.url.includes("localhost"));
      assert.ok(snapshot.result.dom_nodes.some((node) => node.node_ref === frameDocument.owner_node_ref && node.node_name === "IFRAME"));
      assert.notEqual(fieldReference(snapshot), oldRef);
    });
  });
});

for (const layout of ["page", "shadow-root", "same-site", "isolated", "nested-isolated"]) {
test(`${layout} upload uses the observed input and preserves the public tab identity`, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-upload-"));
  try {
    const file = path.join(directory, "report.txt");
    await writeFile(file, "shared room upload");
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request }) => {
        const topLevel = layout === "page" || layout === "shadow-root";
        await page.goto(topLevel ? `${url}field` : url);
        if (layout === "shadow-root") await page.evaluate(() => {
          const host = document.createElement("div");
          document.body.append(host);
          host.attachShadow({ mode: "open" }).append(document.querySelector("label"));
        });
        const frame = topLevel ? page : layout === "nested-isolated"
          ? page.frameLocator("iframe").frameLocator("iframe") : page.frameLocator("iframe");
        const input = frame.getByLabel("Upload");
        await input.waitFor();
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const snapshot = await request("browser.snapshot", target);
        assert.equal(snapshot.ok, true, JSON.stringify(snapshot.error));
        const node = snapshot.result.dom_nodes.find((node) => node.node_name === "INPUT");
        assert.ok(node);
        if (layout.includes("isolated")) assert.match(node.node_ref, /^frame:/, "fixture must use an isolated renderer reference");
        const result = await request("browser.upload", { ...target, node_ref: node.node_ref, file_paths: [file] });
        assert.equal(result.ok, true, JSON.stringify(result.error));
        assert.equal(result.result.target_id, target.target_id);
        assert.equal(result.result.document_id, target.document_id);
        assert.equal(result.result.file_count, 1);
        assert.equal(result.result.total_bytes, 18);
        assert.deepEqual(await input.evaluate(async (input) => ({
          name: input.files[0]?.name, text: await input.files[0]?.text(),
        })), { name: "report.txt", text: "shared room upload" });
      }, { uploadRoots: [directory] });
    }, { fieldMarkup: '<label>Upload<input type="file"></label>', sameSite: layout === "same-site", nested: layout === "nested-isolated" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
}

test("isolated uploads reject another tab, old frame documents, and escaped upload roots", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-upload-"));
  const outside = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-outside-"));
  try {
    const file = path.join(directory, "report.txt");
    const link = path.join(directory, "link.txt");
    const secret = path.join(outside, "secret.txt");
    await writeFile(file, "shared room upload");
    await writeFile(secret, "must not upload");
    await symlink(secret, link);
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request, context }) => {
        await page.goto(url);
        const input = page.frameLocator("iframe").getByLabel("Upload");
        await input.waitFor();
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const nodeRef = await uploadReference(request, target);
        const other = await context.newPage();
        await other.goto(`${url}?other=1`);
        const otherInput = other.frameLocator("iframe").getByLabel("Upload");
        await otherInput.waitFor();
        const otherTarget = (await request("browser.reconcile", { viewport })).result.tabs.find((tab) => tab.target_id !== target.target_id);
        const wrongTab = await request("browser.upload", { ...otherTarget, node_ref: nodeRef, file_paths: [file] });
        assert.equal(wrongTab.ok, false);
        assert.equal(wrongTab.error.code, "stale_element_reference");
        assert.equal(await otherInput.evaluate((input) => input.files.length), 0);
        for (const deniedPath of [secret, link]) {
          const denied = await request("browser.upload", { ...target, node_ref: nodeRef, file_paths: [file, deniedPath] });
          assert.equal(denied.ok, false);
          assert.equal(denied.error.code, "browser_upload_denied");
          assert.equal(await input.evaluate((input) => input.files.length), 0, "failed validation must not upload even the allowed file");
        }
        await page.frames()[1].goto(`http://localhost:${new URL(url).port}/field?revision=2`);
        await input.waitFor();
        const stale = await request("browser.upload", { ...target, node_ref: nodeRef, file_paths: [file] });
        assert.equal(stale.ok, false);
        assert.equal(stale.error.code, "stale_element_reference");
        assert.equal(await input.evaluate((input) => input.files.length), 0);
        const freshRef = await uploadReference(request, target);
        assert.notEqual(freshRef, nodeRef);
        const recovered = await request("browser.upload", { ...target, node_ref: freshRef, file_paths: [file] });
        assert.equal(recovered.ok, true, JSON.stringify(recovered.error));
        assert.equal(await input.evaluate((input) => input.files[0].name), "report.txt");
      }, { uploadRoots: [directory] });
    }, { fieldMarkup: '<label>Upload<input type="file" multiple></label>' });
  } finally {
    await rm(directory, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});

for (const navigation of ["top", "same-site-child", "isolated-child", "isolated-parent"]) {
test(`uploads reject ${navigation} navigation during asynchronous file validation`, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-upload-"));
  try {
    const file = path.join(directory, "report.txt");
    await writeFile(file, "shared room upload");
    let navigate;
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request }) => {
        const startUrl = navigation === "top" ? `${url}field` : url;
        await page.goto(startUrl);
        const input = (navigation === "top" ? page : page.frameLocator("iframe")).getByLabel("Upload");
        await input.waitFor();
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const nodeRef = await uploadReference(request, target);
        navigate = async () => {
          if (navigation.endsWith("-child")) await page.frames()[1].goto(`${page.frames()[1].url()}?new=1`);
          else await page.goto(`${startUrl}?new=1`);
          await input.waitFor();
        };
        const result = await request("browser.upload", { ...target, node_ref: nodeRef, file_paths: [file] });
        assert.equal(result.ok, false);
        assert.equal(result.error.code, navigation === "same-site-child" ? "stale_element_reference" : "stale_document_reference");
        assert.equal(await input.evaluate((input) => input.files.length), 0);
      }, { uploadRoots: [directory], fileSystem: {
        realpath,
        stat: async (candidate) => {
          const metadata = await stat(candidate);
          if (path.basename(candidate) === "report.txt") await navigate();
          return metadata;
        },
      } });
    }, { fieldMarkup: '<label>Upload<input type="file"></label>', sameSite: navigation === "same-site-child" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
}

for (const layout of ["page", "same-site", "isolated"]) {
test(`${layout} uploads reject replaced file inputs`, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-upload-"));
  try {
    const file = path.join(directory, "report.txt");
    await writeFile(file, "shared room upload");
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request }) => {
        await page.goto(layout === "page" ? `${url}field` : url);
        const input = (layout === "page" ? page : page.frameLocator("iframe")).getByLabel("Upload");
        await input.waitFor();
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const nodeRef = await uploadReference(request, target);
        await input.evaluate((input) => { window.detachedInput = input; input.replaceWith(input.cloneNode()); });
        const result = await request("browser.upload", { ...target, node_ref: nodeRef, file_paths: [file] });
        assert.equal(result.ok, false);
        assert.equal(result.error.code, "stale_element_reference");
        assert.equal(await input.evaluate((input) => input.files.length), 0);
        assert.equal(await input.evaluate(() => window.detachedInput.files.length), 0);
        const freshRef = await uploadReference(request, target);
        const recovered = await request("browser.upload", { ...target, node_ref: freshRef, file_paths: [file] });
        assert.equal(recovered.ok, true, JSON.stringify(recovered.error));
        assert.equal(await input.evaluate((input) => input.files[0].name), "report.txt");
      }, { uploadRoots: [directory] });
    }, { fieldMarkup: '<label>Upload<input type="file"></label>', sameSite: layout === "same-site" });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
}

for (const navigation of ["top", "isolated-child", "isolated-parent"]) {
test(`uploads recheck ${navigation} after the renderer inspects the input`, { timeout: 15_000 }, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-upload-"));
  try {
    const file = path.join(directory, "report.txt");
    await writeFile(file, "shared room upload");
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request, browser }) => {
        const startUrl = navigation === "top" ? `${url}field` : url;
        await page.goto(startUrl);
        const input = (navigation === "top" ? page : page.frameLocator("iframe")).getByLabel("Upload");
        await input.waitFor();
        const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const nodeRef = await uploadReference(request, target);
        const connection = await browser.ensureConnection();
        const send = connection.send.bind(connection);
        let navigated = false;
        let fileMutations = 0;
        connection.send = async (method, params, ...rest) => {
          if (method === "DOM.setFileInputFiles") fileMutations += 1;
          const result = await send(method, params, ...rest);
          if (!navigated && method === "Runtime.callFunctionOn" && result?.result?.value === "file") {
            navigated = true;
            if (navigation === "isolated-child") await page.frames()[1].goto(`${page.frames()[1].url()}?new=1`);
            else await page.goto(`${startUrl}?new=1`);
            await input.waitFor();
          }
          return result;
        };
        const rejected = await request("browser.upload", { ...target, node_ref: nodeRef, file_paths: [file] });
        assert.equal(navigated, true);
        assert.equal(rejected.ok, false);
        assert.equal(fileMutations, 0, "never send file paths after the owning document changes");
        assert.equal(rejected.error.code, "stale_document_reference");
        assert.equal(await input.evaluate((node) => node.files.length), 0);
        const current = (await request("browser.reconcile", { viewport })).result.tabs[0];
        const freshRef = await uploadReference(request, current);
        const recovered = await request("browser.upload", { ...current, node_ref: freshRef, file_paths: [file] });
        assert.equal(recovered.ok, true, JSON.stringify(recovered.error));
        assert.equal(fileMutations, 1);
        assert.equal(await input.evaluate((node) => node.files[0].name), "report.txt");
      }, { uploadRoots: [directory] });
    }, { fieldMarkup: '<label>Upload<input type="file"></label>' });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
}

async function uploadReference(request, target) {
  const snapshot = await request("browser.snapshot", target);
  assert.equal(snapshot.ok, true, JSON.stringify(snapshot.error));
  const inputs = snapshot.result.dom_nodes.filter((node) => node.node_name === "INPUT");
  assert.equal(inputs.length, 1);
  return inputs[0].node_ref;
}

test("permissions reach Chrome and update the requested web permission only", async () => {
  await withCrossOriginFixture(async (url) => {
    await withController(async ({ page, request, context }) => {
      await page.goto(url);
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      const other = await context.newPage();
      await other.goto(url.replace("127.0.0.1", "localhost"));
      for (const [permission, descriptor] of [
        ["geolocation", { name: "geolocation" }],
        ["notifications", { name: "notifications" }],
        ["camera", { name: "camera" }],
        ["microphone", { name: "microphone" }],
        ["display-capture", { name: "display-capture" }],
        ["midi", { name: "midi", sysex: false }],
        ["midi-sysex", { name: "midi", sysex: true }],
        ["clipboard-read-write", { name: "clipboard-read" }],
        ["clipboard-sanitized-write", { name: "clipboard-write" }],
        ["local-fonts", { name: "local-fonts" }],
      ]) {
        const otherBefore = await other.evaluate(async (descriptor) => (await navigator.permissions.query(descriptor)).state, descriptor);
        for (const setting of ["granted", "denied", "prompt"]) {
          const result = await request("browser.permission", { ...target, permission, setting });
          assert.equal(result.ok, true, `${permission}/${setting}: ${JSON.stringify(result.error)}`);
          assert.equal(await page.evaluate(async (descriptor) => (await navigator.permissions.query(descriptor)).state, descriptor), setting, permission);
          assert.equal(await other.evaluate(async (descriptor) => (await navigator.permissions.query(descriptor)).state, descriptor), otherBefore, "another origin must be unchanged");
        }
      }
      await page.goto(`${url}field?new-document=1`);
      const stale = await request("browser.permission", { ...target, permission: "geolocation", setting: "granted" });
      assert.equal(stale.ok, false);
      assert.equal(stale.error.code, "stale_document_reference");
      assert.equal(await page.evaluate(async () => (await navigator.permissions.query({ name: "geolocation" })).state), "prompt");
    });
  });
});

test("canceling a download after its tab closes preserves another active download", { timeout: 15_000 }, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-download-"));
  const responses = new Map();
  try {
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request, context }) => {
        try {
        await page.goto(`${url}field?a`);
        const other = await context.newPage();
        await other.goto(`${url}field?b`);
        const reconciled = (await request("browser.reconcile", { viewport })).result;
        const targetA = reconciled.tabs.find((tab) => tab.url.endsWith("?a"));
        const targetB = reconciled.tabs.find((tab) => tab.url.endsWith("?b"));
        assert.equal((await request("browser.downloads.configure", targetA)).ok, true);
        const events = [];
        let cursor = reconciled.event_cursor;
        const pollUntil = async (predicate) => {
          const deadline = Date.now() + 5_000;
          while (!predicate()) {
            assert.ok(Date.now() < deadline, "expected bounded download progress");
            const polled = await request("browser.events.poll", { browser_generation: reconciled.browser_generation, cursor });
            assert.equal(polled.ok, true);
            assert.equal(polled.result.replay_gap, false);
            events.push(...polled.result.events);
            cursor = polled.result.next_cursor;
            if (!predicate()) await new Promise((resolve) => setTimeout(resolve, 20));
          }
        };
        for (const [target, name] of [[targetA, "A"], [targetB, "B"]]) {
          const snapshot = await request("browser.snapshot", target);
          const link = snapshot.result.accessibility_nodes.find((node) => node.role === "link" && node.name === name);
          assert.ok(link);
          const clicked = await request("browser.action", { ...target, node_ref: link.node_ref, action: { kind: "click" } });
          assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
        }
        await pollUntil(() => events.filter((event) => event.kind === "download_started").length === 2);
        const guidA = events.find((event) => event.kind === "download_started" && event.target_id === targetA.target_id).data.guid;
        const guidB = events.find((event) => event.kind === "download_started" && event.target_id === targetB.target_id).data.guid;
        await page.close();
        const cancellation = { browser_generation: reconciled.browser_generation, guid: guidA };
        const stale = await request("browser.downloads.cancel", { ...cancellation, browser_generation: cancellation.browser_generation + 1 });
        assert.equal(stale.ok, false);
        assert.equal(stale.error.code, "stale_browser_generation");
        const unknown = await request("browser.downloads.cancel", { ...cancellation, guid: "not-observed" });
        assert.equal(unknown.ok, false);
        assert.equal(unknown.error.code, "browser_download_not_active");
        const canceled = await request("browser.downloads.cancel", cancellation);
        assert.equal(canceled.ok, true, JSON.stringify(canceled.error));
        assert.equal(canceled.result.cancellation_requested, true);
        await pollUntil(() => events.some((event) => event.kind === "download_progress" && event.data.guid === guidA && event.data.state === "canceled"));
        assert.ok(!events.some((event) => event.kind === "download_progress" && event.data.guid === guidB && event.data.state !== "inProgress"));
        assert.equal(responses.get("/slow?b").destroyed, false);
        responses.get("/slow?b").end("rest");
        await pollUntil(() => events.some((event) => event.kind === "download_progress" && event.data.guid === guidB && event.data.state === "completed"));
        assert.equal(await readFile(path.join(directory, guidB), "utf8"), "x".repeat(1024) + "rest");
        const repeated = await request("browser.downloads.cancel", cancellation);
        assert.equal(repeated.ok, false);
        assert.equal(repeated.error.code, "browser_download_not_active");
        const files = await readdir(directory);
        assert.ok(!files.some((file) => file.startsWith(guidA)), "canceled partial file must be removed");
        } finally {
          for (const response of responses.values()) if (!response.destroyed && !response.writableEnded) response.end("rest");
        }
      }, { downloadDirectory: directory });
    }, {
      download: true,
      fieldMarkup: '<a href="/slow?a">A</a><a href="/slow?b">B</a>',
      downloadHandler: (request, response) => {
        if (!request.url.startsWith("/slow?")) return false;
        responses.set(request.url, response);
        response.writeHead(200, { "content-type": "text/plain", "content-disposition": 'attachment; filename="sample.txt"', "content-length": "1028" });
        response.write("x".repeat(1024));
        return true;
      },
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

for (const boundary of ["mkdir", "realpath", "stat", "statfs"]) {
test(`downloads reject navigation during directory ${boundary} and allow a fresh retry`, { timeout: 10_000 }, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-download-"));
  let navigate;
  let navigated = false;
  const fileSystem = Object.fromEntries(Object.entries({ mkdir, realpath, stat, statfs }).map(([name, operation]) => [name, async (...args) => {
    const result = await operation(...args);
    if (name === boundary && !navigated) {
      navigated = true;
      await navigate();
    }
    return result;
  }]));
  try {
    await withController(async ({ page, request }) => {
      await page.goto("data:text/html,<p>First document</p>");
      const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
      navigate = () => page.goto("data:text/html,<p>Replacement document</p>");
      const stale = await request("browser.downloads.configure", target);
      assert.equal(navigated, true);
      assert.equal(stale.ok, false, "navigation during filesystem work must reject the old document");
      assert.equal(stale.error.code, "stale_document_reference");
      const current = (await request("browser.reconcile", { viewport })).result.tabs[0];
      assert.equal(current.target_id, target.target_id);
      assert.notEqual(current.document_id, target.document_id);
      const fresh = await request("browser.downloads.configure", current);
      assert.equal(fresh.ok, true, JSON.stringify(fresh.error));
      assert.equal(fresh.result.document_id, current.document_id);
    }, { downloadDirectory: directory, fileSystem });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
}

for (const layout of ["page", "same-site", "isolated", "nested-isolated"]) {
for (const late of layout === "page" ? [false] : [false, true]) {
test(`${late ? "late " : ""}${layout} download persists real bytes with tab-attributed progress and a safe filename`, async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-download-"));
  try {
    await withCrossOriginFixture(async (url) => {
      await withController(async ({ page, request }) => {
        await page.goto(layout === "page" ? `${url}field` : url);
        const frame = layout === "page" ? page : layout === "nested-isolated"
          ? page.frameLocator("iframe").frameLocator("iframe") : page.frameLocator("iframe");
        const link = frame.getByRole("link", { name: "Download" });
        await link.waitFor();
        const reconciled = (await request("browser.reconcile", { viewport })).result;
        const target = reconciled.tabs[0];
        const configured = await request("browser.downloads.configure", target);
        assert.equal(configured.ok, true, JSON.stringify(configured.error));
        if (late) {
          await page.locator("iframe").evaluate((frame) => frame.replaceWith(frame.cloneNode()));
          await link.waitFor();
        }
        if (late) {
          await link.evaluate((link) => link.addEventListener("click", () => { link.dataset.activated = "true"; }, { once: true }));
          await link.press("Enter");
          assert.equal(await link.getAttribute("data-activated"), "true", "the fixture link must actually activate");
        }
        else {
          const snapshot = await request("browser.snapshot", target);
          const node = snapshot.result.accessibility_nodes.find((node) => node.role === "link" && node.name === "Download");
          assert.ok(node);
          const clicked = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind: "click" } });
          assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
        }
        let cursor = reconciled.event_cursor;
        const events = [];
        const deadline = Date.now() + 5_000;
        while (!events.some((event) => event.kind === "download_progress" && event.data.state === "completed")) {
          assert.ok(Date.now() < deadline, `download must finish within the bounded fixture timeout: ${JSON.stringify(events.filter((event) => event.kind.startsWith("download_")))}`);
          const polled = await request("browser.events.poll", { browser_generation: reconciled.browser_generation, cursor });
          assert.equal(polled.ok, true, JSON.stringify(polled.error));
          assert.equal(polled.result.replay_gap, false);
          events.push(...polled.result.events);
          cursor = polled.result.next_cursor;
          if (!events.some((event) => event.kind === "download_progress" && event.data.state === "completed")) await new Promise((resolve) => setTimeout(resolve, 10));
        }
        const downloads = events.filter((event) => event.kind.startsWith("download_"));
        const start = downloads.find((event) => event.kind === "download_started");
        assert.ok(start);
        assert.ok(downloads.every((event) => event.target_id === target.target_id), JSON.stringify(downloads));
        assert.equal(start.data.suggested_filename.includes("/"), false);
        assert.equal(start.data.url.includes("fixture-secret"), false);
        assert.match(start.data.guid, /^[0-9a-f-]+$/);
        assert.equal(await readFile(path.join(directory, start.data.guid), "utf8"), "shared room download");
        assert.equal(downloads.at(-1).data.received_bytes, 20);
      }, { downloadDirectory: directory });
    }, { sameSite: layout === "same-site", nested: layout === "nested-isolated", fieldMarkup: '<a href="/download?token=fixture-secret">Download</a>', download: true });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
}
}

// MP-08 / MP-10 / MP-11: credential-free, first-party product regressions.
test("MP-08 P1 handler-backed targets remain document-bound and hit-tested", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    for (const id of ["listener", "inline", "svg", "far"]) {
      const node = snapshot.dom_nodes.find(n => n.attributes.id === id);
      assert.equal(node.clickable, true, `${id} is an observed handler target`);
      const clicked = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind: "click" } });
      assert.equal(clicked.ok, true, JSON.stringify(clicked.error));
      assert.equal(await page.locator(`#${id}`).getAttribute("data-clicks"), "1");
    }
    for (const id of ["inert", "hidden"]) {
      const node = snapshot.dom_nodes.find(n => n.attributes.id === id);
      assert.ok(!node.clickable || !node.rendered);
    }
    const covered = snapshot.dom_nodes.find(n => n.attributes.id === "covered");
    const rejected = await request("browser.action", { ...target, node_ref: covered.node_ref, action: { kind: "click" }, timeout_ms: 100 });
    assert.equal(rejected.ok, false);
    assert.match(rejected.error.message, /obscured/);
    await page.goto(`${url}field?replacement`);
    const stale = await request("browser.action", { ...target, node_ref: covered.node_ref, action: { kind: "click" } });
    assert.equal(stale.error.code, "stale_document_reference");
  }), { fieldMarkup: `<style>.target {display:block;width:160px;height:35px} #cover {position:absolute;inset:0;background:white}</style>
    <span class="target" id="listener">Listener choice</span>
    <div class="target" id="inline" onclick="this.dataset.clicks=Number(this.dataset.clicks||0)+1">Inline choice</div>
    <svg width="160" height="35"><rect id="svg" width="160" height="35" onclick="this.dataset.clicks=Number(this.dataset.clicks||0)+1"/></svg>
    <span id="inert">Inert content</span><div id="hidden" hidden onclick="this.dataset.clicks=1">Hidden</div>
    <div style="position:relative;width:160px;height:35px"><div class="target" id="covered" onclick="this.dataset.clicks=1">Covered</div><div id="cover"></div></div>
    <div style="height:1600px"></div><div class="target" id="far" onclick="this.dataset.clicks=Number(this.dataset.clicks||0)+1">Offscreen choice</div>
    <script>document.getElementById('listener').addEventListener('click', function(){this.dataset.clicks=Number(this.dataset.clicks||0)+1})</script>` });
});

test("MP-08 P2 native selects fill unique labels and values with one event pair", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    const select = snapshot.dom_nodes.find(n => n.attributes.id === "choice");
    const fill = text => request("browser.action", { ...target, node_ref: select.node_ref, action: { kind: "fill", text }, timeout_ms: 100 });
    const selected = await fill("日本");
    assert.equal(selected.ok, true, JSON.stringify(selected.error));
    assert.equal(await page.locator("select").inputValue(), "jp");
    assert.deepEqual(await page.evaluate(() => window.effects), ["input", "change"]);
    for (const [text, code] of [["Same", "browser_selection_ambiguous"], ["Disabled", "browser_selection_disabled"], ["Missing", "browser_selection_not_found"]]) {
      const rejected = await fill(text);
      assert.equal(rejected.error.code, code);
      assert.equal(await page.locator("select").inputValue(), "jp");
    }
    assert.equal((await fill("b")).ok, true);
    assert.equal(await page.locator("select").inputValue(), "b");
    assert.deepEqual(await page.evaluate(() => window.effects), ["input", "change", "input", "change"]);
  }), { fieldMarkup: `<label>Choice<select id="choice"><option value="a">Same</option><option value="b">Same</option><option value="jp">日本</option><option disabled>Disabled</option></select></label>
    <script>window.effects=[];for(const kind of ['input','change'])document.querySelector('select').addEventListener(kind,()=>effects.push(kind))</script>` });
});

test("MP-08 P2 native select reports event-handler rejection without replay", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    const node = snapshot.dom_nodes.find(n => n.attributes.id === "choice");
    const result = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind: "fill", text: "Second" } });
    assert.equal(result.ok, false, "a rejected applied value must not report success");
    assert.equal(result.error.code, "browser_selection_not_applied");
    assert.equal(await page.locator("select").inputValue(), "First");
    assert.deepEqual(await page.evaluate(() => window.effects), ["input", "change"]);
  }), { fieldMarkup: `<select id="choice"><option>First</option><option>Second</option></select><script>window.effects=[];const control=document.querySelector('select');control.addEventListener('input',()=>{effects.push('input');control.value='First'});control.addEventListener('change',()=>effects.push('change'))</script>` });
});

test("MP-08 P2 custom searchable combobox selects observed options and dependent fields", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = () => request("browser.snapshot", target);
    const action = async (name, kind, text) => {
      const current = (await snapshot()).result;
      const node = current.accessibility_nodes.find(node => node.name === name && !node.ignored && current.dom_nodes.some(dom => dom.node_ref === node.node_ref && dom.node_type === 1));
      assert.ok(node, `observed ${name}`);
      const result = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind, ...(text === undefined ? {} : { text }) } });
      assert.equal(result.ok, true, JSON.stringify(result.error));
    };
    await action("Country", "click");
    await action("Country search", "fill", "日本");
    await action("日本", "click");
    assert.equal(await page.locator("#country").textContent(), "日本");
    await action("Region", "fill", "East");
    assert.equal(await page.locator("#region").inputValue(), "east");
    const current = (await snapshot()).result;
    assert.ok(current.accessibility_nodes.some(node => node.role === "combobox" && node.name === "Country"));
    assert.equal(await page.locator("#country").getAttribute("data-effects"), "1");
  }), { fieldMarkup: `<button id="country" role="combobox" aria-label="Country" aria-controls="choices" aria-expanded="false" onclick="document.getElementById('choices').hidden=false;this.setAttribute('aria-expanded','true')">Choose</button>
    <div id="choices" role="listbox" hidden><input aria-label="Country search" oninput="document.getElementById('jp').hidden=!('日本'.includes(this.value))"><div id="jp" role="option" onclick="const c=document.getElementById('country');c.textContent=this.textContent;c.dataset.effects=Number(c.dataset.effects||0)+1;c.setAttribute('aria-expanded','false');document.getElementById('choices').hidden=true;document.getElementById('region').disabled=false">日本</div></div>
    <label>Region<select id="region" disabled><option value="">Choose</option><option value="east">East</option></select></label>` });
});

test("MP-08 P2 multi-select replaces its choice and cancellation never replays delivered events", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request, browser }) => {
    await page.goto(`${url}field`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const snapshot = (await request("browser.snapshot", target)).result;
    const node = snapshot.dom_nodes.find(n => n.attributes.id === "multi");
    const blocked = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind: "fill", text: "Blocked" } });
    assert.equal(blocked.error.code, "browser_selection_disabled");
    assert.deepEqual(await page.evaluate(() => window.effects), []);
    const connection = await browser.ensureConnection();
    const send = connection.send.bind(connection);
    const controller = new AbortController();
    let delivered = 0;
    connection.send = async (method, params, ...rest) => {
      const result = await send(method, params, ...rest);
      if (method === "Runtime.callFunctionOn" && params.functionDeclaration.includes("function selectFunction(")) {
        delivered++;
        controller.abort();
      }
      return result;
    };
    const result = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind: "fill", text: "World" } }, { signal: controller.signal });
    assert.equal(result.ok, true, JSON.stringify(result.error));
    assert.equal(controller.signal.aborted, true);
    assert.equal(delivered, 1);
    assert.deepEqual(await page.locator("#multi").evaluate(n => Array.from(n.selectedOptions, o => o.value)), ["🌍"]);
    assert.deepEqual(await page.evaluate(() => window.effects), ["input", "change"]);
    const revision = (await request("browser.snapshot", target)).result.snapshot_revision;
    assert.ok(revision > snapshot.snapshot_revision);
    const stopped = await request("browser.action", { ...target, node_ref: node.node_ref, action: { kind: "fill", text: "One" } }, { signal: controller.signal });
    assert.equal(stopped.error.code, "browser_action_cancelled");
    assert.deepEqual(await page.evaluate(() => window.effects), ["input", "change"]);
  }), { fieldMarkup: `<label>Multiple<select multiple id="multi"><option selected>One</option><option selected>Two</option><option value="🌍">World</option><optgroup disabled label="Unavailable"><option>Blocked</option></optgroup></select></label>
    <script>window.effects=[];for(const kind of ['input','change'])document.querySelector('select').addEventListener(kind,()=>effects.push(kind))</script>` });
});

for (const layout of ["same-site", "isolated", "nested-isolated"]) {
test(`MP-08 P3 ${layout} rendered text captures only the bound tab and fences frame replacement`, async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request, context }) => {
    await page.goto(url);
    const field = layout === "nested-isolated" ? page.frameLocator("iframe").frameLocator("iframe") : page.frameLocator("iframe");
    await field.getByText("Frame text").waitFor();
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const other = await context.newPage();
    await other.goto("data:text/html,<p>Other tab private content</p>");
    const connection = await browserTextObservation(request, target);
    assert.ok(connection.text.includes("Frame text"));
    assert.ok(!connection.text.includes("Other tab private content"));
    assert.ok(!connection.text.includes("POLLUTION"));
    await page.goto(`${url}field?new-document`);
    const stale = await request("browser.snapshot", { ...target, text_request: { offset: 0 } });
    assert.equal(stale.error.code, "stale_document_reference");
  }), { sameSite: layout === "same-site", nested: layout === "nested-isolated", fieldMarkup: '<p>Frame text</p><script>/* POLLUTION */</script>' });
});
}
async function browserTextObservation(request, target, textRequest = {}) {
  const snapshot = await request("browser.snapshot", { ...target, text_request: { offset: 0, ...textRequest } });
  assert.equal(snapshot.ok, true, JSON.stringify(snapshot.error));
  return snapshot.result.text_page;
}

test("MP-08/MP-10/MP-11 rendered read-only panes expose current public text with paging and redaction", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request, browser }) => {
    await page.goto(`${url}field`);
    const publicText = "Current public row\n".repeat(300) + "Final public row: 日本 😀\n";
    const syntheticSecret = "synthetic-protected-pane-value";
    browser.protectedValues.add(syntheticSecret);
    await page.locator("#public").evaluate((node, value) => { node.value = value; }, publicText);
    await page.locator("#protected").evaluate((node, value) => { node.value = value; }, syntheticSecret);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const first = await browserTextObservation(request, target, { offset: 0, max_bytes: 256 });
    assert.ok(first.text.includes("Current public row"));
    assert.ok(first.next_offset > 0);
    const tail = await browserTextObservation(request, target, { query: "Final public row", max_bytes: 256 });
    assert.ok(tail.text.includes("Final public row: 日本 😀"));
    const redacted = await browserTextObservation(request, target, { query: "[redacted]" });
    assert.ok(redacted.text.includes("[redacted]"));
    for (const excluded of [syntheticSecret, "Hidden pane", "Transparent pane", "Editable draft", "Password fixture", "Old authored row"]) {
      const observation = await browserTextObservation(request, target, { query: excluded });
      assert.equal(observation.text, "", `MP-08/MP-10/MP-11 excluded ${excluded === syntheticSecret ? "protected value" : excluded}`);
    }
  }), { fieldMarkup: '<textarea readonly id="public">Old authored row</textarea><textarea readonly id="protected"></textarea><textarea readonly hidden>Hidden pane</textarea><div style="opacity:0"><textarea readonly>Transparent pane</textarea></div><textarea>Editable draft</textarea><input type="password" value="Password fixture">' });
});

test("MP-08 P3 rendered exact-copy text preserves edge newlines and BR boundaries", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    const reconciled = await request("browser.reconcile", { viewport });
    assert.equal(reconciled.ok, true, JSON.stringify(reconciled.error));
    const target = reconciled.result.tabs[0];
    const result = await request("browser.snapshot", { ...target, text_request: { offset: 0 } });
    assert.equal(result.ok, true, JSON.stringify(result.error));
    assert.equal(result.result.text_page.text, "\n  indented\n\nbefore\n\nafter");
  }), { fieldMarkup: '<pre><span>\n  indented\n\n</span></pre><p>before<br><br>after</p><iframe src="about:blank"></iframe>' });
});

test("MP-08 P3 CSS visibility overrides retain visible text but not transparent text", async () => {
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    assert.equal(await page.locator("#visible").isVisible(), true);
    const reconciled = await request("browser.reconcile", { viewport });
    assert.equal(reconciled.ok, true, JSON.stringify(reconciled.error));
    const text = await browserTextObservation(request, reconciled.result.tabs[0]);
    assert.equal(text.text, "Visible override");
  }), { fieldMarkup: '<p style="visibility:hidden">Hidden text<span id="visible" style="visibility:visible">Visible override</span></p><p style="opacity:0"><span style="visibility:visible">Transparent text</span></p>' });
});

test("MP-08 P3 rendered text excludes non-rendered pollution and pages beyond node bounds", async () => {
  const rows = Array.from({length: 2700}, (_, i) => `<tr><td>Row ${i}</td><td>7</td></tr>`).join("");
  await withCrossOriginFixture(async (url) => withController(async ({ page, request }) => {
    await page.goto(`${url}field`);
    const target = (await request("browser.reconcile", { viewport })).result.tabs[0];
    const first = await request("browser.snapshot", { ...target, text_request: { offset: 0, max_bytes: 256 } });
    assert.equal(first.ok, true, JSON.stringify(first.error));
    assert.ok(first.result.text_page, "bounded text page must be projected");
    const text = first.result.text_page.text;
    assert.ok(text.includes("Visible code"));
    assert.ok(!text.includes("POLLUTION"));
    assert.equal((text.match(/Same person/g) || []).length, 2);
    assert.ok(first.result.text_page.next_offset > 0);
    assert.ok(Buffer.byteLength(text) <= 256);
    const tail = await request("browser.snapshot", { ...target, text_request: { query: "Row 2699", offset: 0, max_bytes: 256 } });
    assert.equal(tail.ok, true, JSON.stringify(tail.error));
    assert.match(tail.result.text_page.text, /Row 2699\t7/);
    assert.ok(!tail.result.text_page.text.includes("private-fixture"));
  }), { fieldMarkup: `<script>/* POLLUTION ${'x'.repeat(5000)} */</script><style>/* POLLUTION */</style>
    <p hidden>POLLUTION hidden</p><p style="visibility:hidden">POLLUTION invisible</p>
    <ul><li><img alt="Same person">Same person</li><li><img alt="Same person">Same person</li></ul>
    <pre>Visible code\n  indented</pre><input type="password" value="private-fixture"><table>${rows}</table>` });
});

async function withCrossOriginFixture(run, {
  sameSite = false, nested = false,
  download = false,
  downloadHandler = sendFixtureDownload,
  fieldMarkup = '<label>Sample<input></label><button onclick="document.querySelector(\'output\').textContent=\'accepted\'">Accept</button><output role="status"></output>',
} = {}) {
  const childServer = createServer((request, response) => {
    if (download && downloadHandler(request, response)) return;
    response.setHeader("content-type", "text/html; charset=utf-8");
    response.end(nested ? `<iframe style="width:500px;height:100px" src="http://127.0.0.1:${server.address().port}/field"></iframe>` : fieldMarkup);
  });
  const server = createServer((request, response) => {
    if (download && downloadHandler(request, response)) return;
    response.setHeader("content-type", "text/html; charset=utf-8");
    response.end(request.url.startsWith("/field") ? fieldMarkup : `<main style="padding:60px"><iframe style="width:600px;height:200px" src="http://${sameSite ? "127.0.0.1" : "localhost"}:${childServer.address().port}/field"></iframe></main>`);
  });
  try {
    await new Promise((resolve) => childServer.listen(0, "127.0.0.1", resolve));
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    await run(`http://127.0.0.1:${server.address().port}/`);
  } finally {
    for (const fixtureServer of [server, childServer]) {
      fixtureServer.closeAllConnections();
      await new Promise((resolve, reject) => fixtureServer.close((error) => error ? reject(error) : resolve()));
    }
  }
}

function sendFixtureDownload(request, response) {
  if (!request.url.startsWith("/download")) return false;
  response.writeHead(200, { "content-type": "text/plain", "content-disposition": 'attachment; filename="../../report.txt"' });
  response.end("shared room download");
  return true;
}

function fieldReference(response) {
  assert.equal(response.ok, true, JSON.stringify(response.error));
  const fields = response.result.accessibility_nodes.filter((node) => node.role === "textbox" && node.name.trim() === "Sample");
  assert.equal(fields.length, 1);
  return fields[0].node_ref;
}

async function withController(run, clientOptions = {}) {
  const profile = await mkdtemp(path.join(os.tmpdir(), "chariox-controller-browser-"));
  let context;
  let browser;
  try {
    context = await chromium.launchPersistentContext(profile, {
      ...browserLaunchOptions,
      args: [...browserLaunchOptions.args],
    });
    const port = Number((await readFile(path.join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0]);
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535);
    browser = new BrowserCdpClient({ ...clientOptions, debuggerEndpoint: `http://127.0.0.1:${port}` });
    const page = context.pages()[0] ?? await context.newPage();
    page.setDefaultTimeout(10_000);
    let nextId = 0;
    const request = (method, params, options = {}) => handleBrowserControllerRequest({ id: ++nextId, method, params }, { browser, ...options });
    await run({ page, request, context, browser });
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
