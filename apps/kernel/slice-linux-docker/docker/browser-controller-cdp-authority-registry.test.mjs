import assert from "node:assert/strict";
import test from "node:test";

import { BrowserCdpAuthorityRegistry } from "./browser-controller-cdp-authority-registry.mjs";

test("rotates browser, target, and document authority while fencing stale sessions", () => {
  const registry = new BrowserCdpAuthorityRegistry();
  const first = registry.begin({ name: "first" });
  assert.equal(registry.commit(first), true);
  registry.observeTarget(first, "target-a", "endpoint-a");
  registry.setSession(first, "target-a", "session-a");
  registry.setDocument(first, "target-a", "document-a");
  const old = registry.getTarget(first, "target-a");

  registry.retireCurrent();
  const second = registry.begin({ name: "second" });
  assert.equal(registry.commit(second), true);
  registry.observeTarget(second, "target-a", "endpoint-b");
  registry.setSession(second, "target-a", "session-b");
  registry.setDocument(second, "target-a", "document-b");
  const fresh = registry.getTarget(second, "target-a");

  assert.equal(second.browserGeneration, first.browserGeneration + 1);
  assert.equal(fresh.targetGeneration, 1);
  assert.equal(fresh.documentGeneration, 1);
  assert.equal(registry.isCurrent(first), false);
  assert.equal(registry.isSessionCurrent(first, "target-a", "session-a"), false);
  assert.equal(
    registry.isTargetCurrent(second, "target-a", {
      targetGeneration: fresh.targetGeneration,
      documentGeneration: fresh.documentGeneration,
      documentId: "document-b",
      sessionId: "session-b",
    }),
    true,
  );
  assert.equal(registry.isTargetCurrent(second, "target-a", { documentId: old.documentId }), false);
});

test("fences endpoint replacement and preserves the replacement across stale observations", () => {
  const registry = new BrowserCdpAuthorityRegistry();
  const authority = registry.begin({ name: "connection" });
  registry.commit(authority);
  const first = registry.observeTarget(authority, "target-a", "endpoint-a");
  registry.setSession(authority, "target-a", "session-a");
  registry.setDocument(authority, "target-a", "document-a");

  const rotated = registry.observeTarget(authority, "target-a", "endpoint-b");
  assert.equal(rotated.rotated, true);
  assert.equal(rotated.targetGeneration, first.targetGeneration + 1);
  assert.equal(rotated.sessionId, null);
  assert.equal(rotated.documentId, null);
  assert.equal(registry.isSessionCurrent(authority, "target-a", "session-a"), false);

  const stale = registry.observeTarget(authority, "target-a", "endpoint-a");
  assert.equal(stale.rotated, false);
  assert.equal(stale.stale, true);
  assert.equal(stale.targetGeneration, rotated.targetGeneration);
  assert.equal(registry.getTarget(authority, "target-a").sessionId, null);
});
