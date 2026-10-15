// MP-08/MP-11: the kernel's tab-groups component extension.
import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { ChromeTabGroups, TAB_GROUPS_EXTENSION_ID, writeTabGroupsExtension } from "./kernel-browser-tab-groups.mjs";

test("MP-11 the extension id derives from its public key and asks only for tabs, groups and target ids", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-tab-groups-"));
  try {
    const manifest = JSON.parse(await readFile(path.join(await writeTabGroupsExtension(root), "manifest.json"), "utf8"));
    const id = createHash("sha256").update(Buffer.from(manifest.key, "base64")).digest("hex").slice(0, 32).replace(/[0-9a-f]/g, c => String.fromCharCode(97 + parseInt(c, 16)));
    assert.equal(id, TAB_GROUPS_EXTENSION_ID);
    assert.deepEqual(manifest.permissions, ["tabs", "tabGroups", "debugger"]);
    assert.equal(manifest.host_permissions, undefined); assert.equal(manifest.content_scripts, undefined);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("MP-08 one native group per agent label; unchanged groups are not re-sent", async () => {
  const sent = [];
  const connection = { async send(method, params) {
    sent.push({ method, params });
    if (method === "Extensions.loadUnpacked") return { id: TAB_GROUPS_EXTENSION_ID };
    if (method === "Target.getTargets") return { targetInfos: [{ type: "service_worker", targetId: "sw", url: `chrome-extension://${TAB_GROUPS_EXTENSION_ID}/sw.js` }] };
    if (method === "Target.attachToTarget") return { sessionId: "s" };
    if (method === "Runtime.evaluate") return { result: { value: true } };
    return {};
  } };
  const groups = new ChromeTabGroups(connection);
  await groups.load("/k/tab-groups-extension");
  assert.equal(await groups.sync([{ label: "Mara", target_ids: ["t2", "t1"] }]), true);
  assert.equal(await groups.sync([{ label: "Mara", target_ids: ["t1", "t2"] }]), false, "same groups, no evaluate");
  const evaluated = sent.filter(call => call.method === "Runtime.evaluate").map(call => call.params.expression);
  assert.equal(evaluated.length, 1);
  assert.match(evaluated[0], /^charioxTabs\.sync\(\[\{"target_ids":\["t1","t2"\],"title":"Mara","color":"[a-z]+"\}\]\)$/);
  await assert.rejects(new ChromeTabGroups({ send: async () => ({ id: "other" }) }).load("/k"), /unexpected kernel tab-groups extension id/);
});

// Coordinator 09:41Z: the person removes or disables "Chariox tab groups" in chrome://extensions.
test("MP-08/MP-11 a removed tab-groups extension is loaded again; until then it fails closed", async () => {
  let installed = true, loads = 0, uninstalls = 0, failLoad = false;
  const connection = { async send(method, params) {
    if (method === "Extensions.loadUnpacked") { loads++; if (failLoad) throw new Error("load refused"); assert.equal(params.path, "/k/tab-groups-extension"); installed = true; return { id: TAB_GROUPS_EXTENSION_ID }; }
    if (method === "Target.getTargets") return { targetInfos: installed ? [{ type: "service_worker", targetId: "sw", url: `chrome-extension://${TAB_GROUPS_EXTENSION_ID}/sw.js` }] : [] };
    if (method === "Target.attachToTarget") return { sessionId: "s" };
    if (method === "Extensions.uninstall") { uninstalls++; if (!installed) throw new Error("not installed"); installed = false; return {}; }
    if (method === "Runtime.evaluate") { if (!installed) throw new Error("No session with given id"); return { result: { value: "target-b" } }; }
    return {};
  } };
  const groups = new ChromeTabGroups(connection);
  await groups.load("/k/tab-groups-extension");
  assert.equal(await groups.active(), "target-b");
  installed = false;
  assert.equal(await groups.active(), "target-b", "removed: the kernel reloads it and answers");
  assert.equal(loads, 2); assert.equal(uninstalls, 1, "a disabled copy is removed first (loading does not re-enable it)");
  assert.equal(groups.signature, "[]", "groups are sent again to the new copy");
  installed = false; failLoad = true;
  await assert.rejects(groups.active(), /load refused|unavailable/, "not back: band input fails closed");
});

// Coordinator 12:22Z: one browser window. Real Chrome 155 RED/GREEN: evidence browserux/r8/window-fold-*.json.
test("MP-08 a Tab in a second tabbed window folds into the oldest one, also after a drag", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-tab-groups-"));
  try {
    const source = await readFile(path.join(await writeTabGroupsExtension(root), "sw.js"), "utf8");
    const listeners = {}, calls = [];
    let dragging = 2;
    const tabs = new Map([[1, { id: 1, windowId: 10 }], [2, { id: 2, windowId: 11 }], [3, { id: 3, windowId: 12 }]]);
    const chrome = {
      runtime: { getPlatformInfo() {} },
      windows: { getAll: async () => [{ id: 12, type: "popup" }, { id: 11 }, { id: 10 }].filter(window => !window.type), update: async (id, value) => calls.push(["focus", id, value]) },
      tabs: {
        onCreated: { addListener: listener => { listeners.created = listener; } },
        onAttached: { addListener: listener => { listeners.attached = listener; } },
        get: async id => tabs.get(id),
        move: async (id, value) => { if (dragging-- > 0) throw new Error("Tabs cannot be edited right now (user may be dragging a tab)."); calls.push(["move", id, value]); },
        update: async (id, value) => calls.push(["update", id, value]),
      },
    };
    const timers = { setInterval() {}, setTimeout: (callback) => queueMicrotask(callback) };
    new Function("chrome", "setInterval", "setTimeout", "globalThis", source)(chrome, timers.setInterval, timers.setTimeout, {});
    listeners.created(tabs.get(1)); listeners.created(tabs.get(3));
    await listeners.attached(2);
    await new Promise(resolve => setTimeout(resolve, 20));
    assert.deepEqual(calls, [["move", 2, { windowId: 10, index: -1 }], ["update", 2, { active: true }], ["focus", 10, { focused: true }]],
      "only the second normal window's Tab moves (after the drag ends); the main window's and the popup's stay");
  } finally { await rm(root, { recursive: true, force: true }); }
});
