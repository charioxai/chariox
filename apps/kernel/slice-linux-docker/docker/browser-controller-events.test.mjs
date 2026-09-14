import assert from "node:assert/strict";
import test from "node:test";

import { BrowserEventJournal } from "./browser-controller-events.mjs";

test("event journal maps CDP events without retaining sensitive payloads", () => {
  const journal = new BrowserEventJournal({ capacity: 10 });
  const context = {
    browserGeneration: 2,
    targetIdForSession: (sessionId) => sessionId === "session-a" ? "target-a" : null,
    documentIdForTarget: (targetId) => targetId === "target-a" ? "loader-a" : null,
  };

  journal.recordCdp({
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    params: {
      type: "error",
      args: [{ value: "secret-canary" }],
    },
  }, context);
  journal.recordCdp({
    method: "Network.requestWillBeSent",
    sessionId: "session-a",
    params: {
      requestId: "request-1",
      type: "Fetch",
      request: {
        method: "POST",
        url: "https://user:pass@example.test/path?q=secret#fragment",
        headers: { authorization: "Bearer secret-canary" },
        postData: "secret-canary",
      },
    },
  }, context);
  journal.recordCdp({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: {
      type: "prompt",
      message: "secret-canary",
      defaultPrompt: "secret-canary",
    },
  }, context);
  journal.recordCdp({
    method: "Network.loadingFailed",
    sessionId: "session-a",
    params: {
      requestId: "request-1",
      type: "Fetch",
      errorText: `failure ${"secret-canary"}`,
      canceled: false,
    },
  }, context);

  const replay = journal.poll({ cursor: 0, limit: 10, browserGeneration: 2 });
  assert.equal(replay.replay_gap, false);
  assert.deepEqual(replay.events.map((event) => event.kind), [
    "console",
    "network_request",
    "dialog_opened",
    "network_failed",
  ]);
  assert.deepEqual(replay.events[0].data, { console_type: "error", argument_count: 1 });
  assert.deepEqual(replay.events[1].data, {
    request_id: "request-1",
    method: "POST",
    url: "https://example.test/path",
    resource_type: "Fetch",
  });
  assert.deepEqual(replay.events[2].data, {
    dialog_type: "prompt",
    has_message: true,
    has_default_prompt: true,
  });
  assert.deepEqual(replay.events[3].data, {
    request_id: "request-1",
    error_text: "[redacted]",
    canceled: false,
    resource_type: "Fetch",
  });
  assert.equal(JSON.stringify(replay).includes("secret-canary"), false);
  assert.equal(JSON.stringify(replay).includes("authorization"), false);
});

test("event journal bounds replay and reports cursor or generation gaps", () => {
  const journal = new BrowserEventJournal({ capacity: 2 });
  const context = {
    browserGeneration: 3,
    targetIdForSession: () => "target-a",
    documentIdForTarget: () => "loader-a",
  };
  for (const type of ["log", "warning", "error"]) {
    journal.recordCdp({
      method: "Runtime.consoleAPICalled",
      sessionId: "session-a",
      params: { type, args: [] },
    }, context);
  }

  assert.deepEqual(journal.poll({ cursor: 0, limit: 10, browserGeneration: 3 }), {
    browser_generation: 3,
    events: [],
    next_cursor: 3,
    replay_gap: true,
  });
  const current = journal.poll({ cursor: 1, limit: 1, browserGeneration: 3 });
  assert.equal(current.replay_gap, false);
  assert.equal(current.events.length, 1);
  assert.equal(current.events[0].event_id, 2);
  assert.equal(current.next_cursor, 2);
  assert.equal(
    journal.poll({ cursor: 3, limit: 10, browserGeneration: 2 }).replay_gap,
    true,
  );
});

test("event journal maps page, target, download, and crash lifecycle events", () => {
  const journal = new BrowserEventJournal({ capacity: 20 });
  const context = {
    browserGeneration: 4,
    targetIdForSession: () => "target-a",
    targetIdForFrame: () => "target-a",
    targetIdForDownload: () => "target-a",
    documentIdForTarget: () => "loader-a",
  };
  const messages = [
    { method: "Target.targetCreated", params: { targetInfo: { type: "page", targetId: "target-b", url: "https://b.test/?secret=1" } } },
    { method: "Page.frameNavigated", sessionId: "session-a", params: { frame: { id: "frame-a", loaderId: "loader-b", url: "https://a.test/next?secret=1" } } },
    { method: "Page.loadEventFired", sessionId: "session-a", params: {} },
    { method: "Browser.downloadWillBegin", params: { guid: "guid-a", url: "https://a.test/file?token=secret", suggestedFilename: "report.txt" } },
    { method: "Browser.downloadProgress", params: { guid: "guid-a", state: "completed", receivedBytes: 12, totalBytes: 12 } },
    { method: "Inspector.targetCrashed", sessionId: "session-a", params: {} },
    { method: "Chariox.browserDisconnected", params: {} },
  ];
  for (const message of messages) journal.recordCdp(message, context);

  const replay = journal.poll({ cursor: 0, limit: 20, browserGeneration: 4 });
  assert.deepEqual(replay.events.map((event) => event.kind), [
    "target_created",
    "page_navigated",
    "page_loaded",
    "download_started",
    "download_progress",
    "target_crashed",
    "browser_disconnected",
  ]);
  assert.equal(JSON.stringify(replay).includes("secret"), false);
  assert.equal(replay.events[1].document_id, "loader-b");
  assert.deepEqual(replay.events[4].data, {
    guid: "guid-a",
    state: "completed",
    received_bytes: 12,
    total_bytes: 12,
  });
  assert.equal(replay.events[3].target_id, "target-a");
  assert.equal(replay.events[4].target_id, "target-a");
  assert.deepEqual(replay.events[5].data, {
    status: "crashed",
    error_code: null,
  });
});
