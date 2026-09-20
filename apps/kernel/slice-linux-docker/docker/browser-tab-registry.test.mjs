import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserTabRegistry,
  DEFAULT_VIEWPORT,
  ERROR_CODES,
  REGISTRY_LIMITS,
  VIEWPORT_LIMITS,
} from "./browser-tab-registry.mjs";

const target = (targetId, websocketUrl = `ws://127.0.0.1:9222/devtools/page/${targetId}`) => ({
  target_id: targetId,
  target_type: "page",
  websocket_url: websocketUrl,
});

const errorCode = (callback) => {
  assert.throws(callback, (error) => Boolean(error?.code));
  try {
    callback();
  } catch (error) {
    return error.code;
  }
  return undefined;
};

test("keeps opaque tab IDs stable across target refresh reordering", () => {
  const registry = new BrowserTabRegistry();
  const first = registry.reconcile(1, [target("beta"), target("alpha")]);
  const firstIds = new Map(first.tabs.map((tab) => [tab.target_id, tab.tab_id]));
  const refreshed = registry.reconcile(1, [target("alpha"), target("beta")]);
  const refreshedIds = new Map(refreshed.tabs.map((tab) => [tab.target_id, tab.tab_id]));

  assert.deepEqual(refreshedIds, firstIds);
  assert.equal(refreshed.tabs[0].target_id, "alpha");
  assert.ok([...refreshedIds.values()].every((tabId) => !tabId.includes("alpha") && !tabId.includes("beta")));
});

test("rotates tab authority when a target endpoint changes in place", () => {
  const registry = new BrowserTabRegistry();
  const original = registry.reconcile(1, [target("page-1")]).tabs[0];
  const replacementUrl = "ws://127.0.0.1:9222/devtools/page/page-1-replacement";

  const replacement = registry.reconcile(1, [target("page-1", replacementUrl)]);

  assert.deepEqual(replacement.detached, [original]);
  assert.equal(replacement.added.length, 1);
  assert.notEqual(replacement.added[0].tab_id, original.tab_id);
  assert.equal(replacement.added[0].target_generation, original.target_generation + 1);
  assert.equal(replacement.tabs[0].target_id, original.target_id);
  assert.equal(errorCode(() => registry.resolveTarget(original.tab_id)), ERROR_CODES.TAB_INVALIDATED);
  assert.equal(registry.resolveTarget(replacement.added[0].tab_id).websocket_url, replacementUrl);
});

test("suppresses duplicate CDP targets deterministically", () => {
  const registry = new BrowserTabRegistry();
  const result = registry.reconcile(1, [target("same", "ws://127.0.0.1:9222/z"), target("same", "ws://127.0.0.1:9222/a")]);

  assert.equal(result.duplicates_suppressed, 1);
  assert.equal(result.tabs.length, 1);
  assert.equal(registry.resolveTarget(result.tabs[0].tab_id).websocket_url, "ws://127.0.0.1:9222/a");
});

test("invalidates a detached tab and allocates a new target generation on reappearance", () => {
  const registry = new BrowserTabRegistry();
  const original = registry.reconcile(1, [target("page-1")]).tabs[0];
  registry.detachTarget(1, "page-1");
  assert.equal(errorCode(() => registry.getTab(original.tab_id)), ERROR_CODES.TAB_INVALIDATED);

  const replacement = registry.reconcile(1, [target("page-1")]).tabs[0];
  assert.notEqual(replacement.tab_id, original.tab_id);
  assert.equal(replacement.target_generation, original.target_generation + 1);
  assert.equal(errorCode(() => registry.resolveTarget(original.tab_id)), ERROR_CODES.TAB_INVALIDATED);
});

test("binds tab identities to browser generations", () => {
  const registry = new BrowserTabRegistry();
  const oldTab = registry.reconcile(1, [target("page-1")]).tabs[0];
  const next = registry.reconcile(2, [target("page-1")]);

  assert.equal(next.tabs[0].generation, 2);
  assert.notEqual(next.tabs[0].tab_id, oldTab.tab_id);
  assert.equal(errorCode(() => registry.getTab(oldTab.tab_id)), ERROR_CODES.STALE_GENERATION);
  assert.equal(errorCode(() => registry.reconcile(1, [target("page-1")])), ERROR_CODES.STALE_GENERATION);
});

test("transfers canonical viewport ownership with an explicit version", () => {
  const registry = new BrowserTabRegistry();
  const claimed = registry.claimViewport("controller-a", 0);
  assert.equal(claimed.owner_id, "controller-a");
  assert.equal(claimed.version, 1);

  const transferred = registry.transferViewport("controller-a", "controller-b", claimed.version);
  assert.equal(transferred.owner_id, "controller-b");
  assert.equal(transferred.version, 2);
  assert.equal(errorCode(() => registry.resizeViewport("controller-a", transferred.version, 1280, 800)), ERROR_CODES.VIEWPORT_OWNER_MISMATCH);
});

test("rejects stale resize versions and permits an exact retry", () => {
  const registry = new BrowserTabRegistry();
  registry.claimViewport("controller", 0);
  const changed = registry.resizeViewport("controller", 1, 1440, 900);
  assert.equal(changed.version, 2);
  assert.equal(changed.changed, true);

  const retry = registry.resizeViewport("controller", 1, 1440, 900);
  assert.equal(retry.idempotent, true);
  assert.equal(retry.version, 2);
  assert.equal(errorCode(() => registry.resizeViewport("controller", 1, 1600, 900)), ERROR_CODES.STALE_VIEWPORT_VERSION);
});

test("enforces viewport bounds, owner, and version contracts", () => {
  const registry = new BrowserTabRegistry();
  assert.deepEqual(registry.getViewport(), { ...DEFAULT_VIEWPORT, owner_id: null, version: 0 });
  assert.equal(errorCode(() => registry.resizeViewport("controller", 0, 1280, 800)), ERROR_CODES.VIEWPORT_OWNER_REQUIRED);
  registry.claimViewport("controller", 0);
  assert.equal(errorCode(() => registry.resizeViewport("controller", 1, VIEWPORT_LIMITS.minWidth - 1, 800)), ERROR_CODES.VIEWPORT_BOUNDS);
  assert.equal(errorCode(() => registry.resizeViewport("controller", 0, 1280, 800)), ERROR_CODES.STALE_VIEWPORT_VERSION);
  assert.equal(errorCode(() => registry.resizeViewport("other", 1, 1280, 800)), ERROR_CODES.VIEWPORT_OWNER_MISMATCH);
});

test("reconciles reconnects without detaching or duplicating tabs", () => {
  const registry = new BrowserTabRegistry();
  const initial = registry.reconcile(1, [target("one"), target("two")]);
  const reconnect = registry.reconcileReconnect(1, [target("two")]);
  const secondReconnect = registry.reconcileReconnect(1, [target("one"), target("two")]);

  assert.equal(reconnect.tabs.length, 2);
  assert.deepEqual(new Set(reconnect.tabs.map((tab) => tab.tab_id)), new Set(initial.tabs.map((tab) => tab.tab_id)));
  assert.equal(secondReconnect.added.length, 0);
  assert.equal(secondReconnect.tabs.length, 2);
});

test("serializes deterministic state without websocket URLs or sensitive page data", () => {
  const registry = new BrowserTabRegistry();
  registry.reconcile(1, [target("z"), target("a")]);
  const serialized = registry.serialize();
  assert.equal(serialized, registry.serialize());
  assert.deepEqual(Object.keys(JSON.parse(serialized)), ["generation", "tabs", "viewport"]);
  assert.ok(!serialized.includes("websocket"));
  assert.ok(!serialized.includes("127.0.0.1"));
  assert.ok(!serialized.includes("password"));
  assert.deepEqual(registry.listTabs().map((tab) => tab.target_id), ["a", "z"]);
});

test("keeps same-version viewport resize idempotent", () => {
  const registry = new BrowserTabRegistry();
  registry.claimViewport({ owner_id: "controller", version: 0 });
  const first = registry.resizeViewport({ owner_id: "controller", version: 1, width: 1280, height: 800 });
  const second = registry.resizeViewport({ owner_id: "controller", version: 1, width: 1280, height: 800 });

  assert.equal(first.version, 1);
  assert.equal(first.changed, false);
  assert.equal(second.idempotent, true);
  assert.equal(second.version, first.version);
});

test("bounds retired state and never reuses an evicted tab handle", () => {
  const registry = new BrowserTabRegistry();
  const retiredHandles = [];
  let current = registry.reconcile(1, [target("churn-0")]).tabs[0];
  const churnCount = REGISTRY_LIMITS.maxRetiredRecords + REGISTRY_LIMITS.maxTombstones + 8;

  for (let index = 1; index <= churnCount; index += 1) {
    retiredHandles.push(current.tab_id);
    current = registry.reconcile(1, [target(`churn-${index}`)]).tabs[0];
  }

  const generationHandles = [];
  const generationCount = REGISTRY_LIMITS.maxTargetEpochs + 8;
  for (let generation = 2; generation <= generationCount + 1; generation += 1) {
    generationHandles.push(current.tab_id);
    current = registry.reconcile(generation, [target("restarted")]).tabs[0];
  }

  const retiredRecordCount = [...registry._recordsByTabId.values()]
    .filter((record) => !record.active)
    .length;
  assert.ok(retiredRecordCount <= REGISTRY_LIMITS.maxRetiredRecords);
  assert.ok(registry._recordsByTabId.size <= REGISTRY_LIMITS.maxRetiredRecords + registry._activeByTarget.size);
  assert.ok(registry._tombstones.size <= REGISTRY_LIMITS.maxTombstones);
  assert.ok(registry._targetEpochs.size <= REGISTRY_LIMITS.maxTargetEpochs);
  assert.equal(registry.listTabs().length, 1);
  assert.equal(registry.listTabs()[0].tab_id, current.tab_id);
  assert.ok(![...retiredHandles, ...generationHandles].includes(current.tab_id));

  for (const tabId of [retiredHandles[0], retiredHandles.at(-1), generationHandles[0], generationHandles.at(-1)]) {
    assert.throws(
      () => registry.getTab(tabId),
      (error) => [ERROR_CODES.TARGET_NOT_FOUND, ERROR_CODES.TAB_INVALIDATED, ERROR_CODES.STALE_GENERATION].includes(error.code),
    );
  }
});
