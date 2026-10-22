// MP-08/MP-10: real Chromium regression for dialog openings (protocol 569).
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { connectCdpPipe } from "./kernel-browser-cdp-pipe.mjs";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

const viewport = { css_width: 1280, css_height: 720, device_scale_factor: 1, desktop_pixel_width: 1280, desktop_pixel_height: 720 };
const until = async (check) => { for (let attempt = 0; attempt < 200; attempt += 1) { const value = check(); if (value) return value; await new Promise((resolve) => setTimeout(resolve, 25)); } assert.fail("timed out"); };

// Two viewers see the same confirm. One answers; the page asks the same question again in the same document.
// The other's delayed answer to the first must not answer the second, which it never saw.
test("a delayed answer to a dialog opening never answers the identical one after it", { timeout: 60000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), "mirror-dialogs-"));
  const child = spawn(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, [`--user-data-dir=${root}/chrome`, "--remote-debugging-pipe", "--headless=new", "--no-sandbox", "about:blank"], { stdio: ["ignore", "ignore", "ignore", "pipe", "pipe"] });
  const connection = connectCdpPipe(child.stdio[3], child.stdio[4], 10000);
  const browser = new BrowserCdpClient({ connectionFactory: () => connection });
  try {
    const state = await browser.reconcile(viewport);
    const tab = state.tabs[0];
    const { sessionId } = await connection.send("Target.attachToTarget", { targetId: tab.target_id, flatten: true });
    await connection.send("Runtime.evaluate", { expression: "setTimeout(() => { window.answers = [confirm('Delete it?'), confirm('Delete it?')]; }, 0)" }, sessionId);
    const first = await until(() => browser.openDialogs.get(tab.target_id));
    const answer = (opening, action) => browser.handleDialog({ target_id: tab.target_id, document_id: tab.document_id, action, opening });
    await answer(first.opening, "accept");
    const second = await until(() => { const open = browser.openDialogs.get(tab.target_id); return open?.opening !== first.opening && open; });
    assert.equal(second.message, first.message, "the page asked the same question again");
    await assert.rejects(answer(first.opening, "dismiss"), (error) => error.code === "stale_dialog_reference");
    await assert.rejects(answer(first.opening, "accept"), (error) => error.code === "stale_dialog_reference");
    assert.equal(browser.openDialogs.get(tab.target_id)?.opening, second.opening, "the second dialog is still waiting for an answer");
    await answer(second.opening, "dismiss");
    await until(() => browser.openDialogs.get(tab.target_id) === null);
    const { result } = await connection.send("Runtime.evaluate", { expression: "window.answers", returnByValue: true }, sessionId);
    assert.deepEqual(result.value, [true, false], "each dialog got the answer given for it");
  } finally {
    await connection.send("Browser.close").catch(() => {});
    for (let attempt = 0; attempt < 50 && child.exitCode === null && child.signalCode === null; attempt += 1) await new Promise((resolve) => setTimeout(resolve, 100));
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    await rm(root, { recursive: true, force: true });
  }
});
