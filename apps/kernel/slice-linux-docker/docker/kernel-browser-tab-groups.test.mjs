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
