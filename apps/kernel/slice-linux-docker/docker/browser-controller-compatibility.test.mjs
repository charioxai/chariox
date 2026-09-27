import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserCompatibilityError,
  navigateBrowser,
  waitForBrowserState,
} from "./browser-controller-compatibility.mjs";

test("legacy navigation stays document-bound and returns the new loader identity", async () => {
  const calls = [];
  const connection = {
    send: async (method, params, sessionId) => {
      calls.push({ method, params, sessionId });
      if (method === "Page.getFrameTree") {
        return { frameTree: { frame: { loaderId: "loader-a" } } };
      }
      return { loaderId: "loader-b" };
    },
  };

  const result = await navigateBrowser({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    url: "https://example.test/settings",
  });

  assert.deepEqual(result, {
    target_id: "target-a",
    document_id: "loader-b",
    url: "https://example.test/settings",
  });
  assert.deepEqual(calls[1], {
    method: "Page.navigate",
    params: { url: "https://example.test/settings" },
    sessionId: "session-a",
  });
});

test("legacy selector and idle waits poll the same persistent document", async () => {
  let now = 0;
  const evaluations = [];
  const values = [false, true, true];
  const connection = {
    send: async (method, params) => {
      if (method === "Page.getFrameTree") {
        return { frameTree: { frame: { loaderId: "loader-a" } } };
      }
      evaluations.push(params.expression);
      return { result: { value: values.shift() } };
    },
  };
  const common = {
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    timeoutMs: 500,
    now: () => now,
    sleep: async (milliseconds) => {
      now += milliseconds;
    },
  };

  const selector = await waitForBrowserState({
    ...common,
    kind: "selector",
    selector: "#settings",
  });
  const idle = await waitForBrowserState({ ...common, kind: "idle" });

  assert.equal(selector.kind, "selector");
  assert.equal(selector.elapsed_ms, 50);
  assert.equal(idle.kind, "idle");
  assert.match(evaluations[0], /#settings/);
  assert.equal(evaluations.at(-1), "document.readyState === 'complete'");
});

test("legacy compatibility calls reject unsafe inputs and stale documents", async () => {
  const connection = {
    send: async () => ({ frameTree: { frame: { loaderId: "loader-new" } } }),
  };

  await assert.rejects(
    navigateBrowser({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      url: "file:///etc/passwd",
    }),
    (error) => error instanceof BrowserCompatibilityError && error.code === "browser_navigation_invalid",
  );
  await assert.rejects(
    waitForBrowserState({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      kind: "idle",
      timeoutMs: 500,
    }),
    (error) => error instanceof BrowserCompatibilityError && error.code === "stale_document_reference",
  );
});
