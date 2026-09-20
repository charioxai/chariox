import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserEventError,
  BrowserEventJournal,
  boundedSerialize,
  redactHeaders,
  sanitizeUrl,
} from "./browser-controller-events.mjs";

const SECRET = "canary-auth-token-should-never-escape";

function byteLength(value) {
  return new TextEncoder().encode(value).byteLength;
}

function context(browserGeneration = 7) {
  return {
    browserGeneration,
    actorId: "actor-7",
    tabIdForTarget: (targetId) => (targetId ? `tab-for-${targetId}` : null),
    actorIdForTarget: (targetId) => (targetId ? `actor-for-${targetId}` : null),
    targetIdForSession: (sessionId) => (sessionId === "session-a" ? "target-a" : null),
    targetIdForFrame: (frameId) => (frameId === "frame-a" ? "target-a" : null),
    targetIdForDownload: (guid) => (guid === "download-a" ? "target-a" : null),
    documentIdForTarget: (targetId) => (targetId === "target-a" ? "document-a" : null),
  };
}

test("maps console, network, page, dialog, download, and lifecycle events with attribution", () => {
  const journal = new BrowserEventJournal({ maxEvents: 32, maxBytes: 32_768 });
  let currentDocument = "document-a";
  const source = {
    ...context(),
    documentIdForTarget: (targetId) => (targetId === "target-a" ? currentDocument : null),
  };

  journal.recordCdp({
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    eventId: "console-1",
    params: { type: "error", args: Array.from({ length: 4 }, () => ({ value: SECRET })) },
  }, source);
  journal.recordCdp({
    method: "Network.requestWillBeSent",
    sessionId: "session-a",
    eventId: "request-1",
    params: {
      requestId: "request-1",
      type: "Fetch",
      request: {
        method: "POST",
        url: `https://user:pass@example.test/path?q=${SECRET}#fragment`,
        headers: { authorization: `Bearer ${SECRET}`, cookie: SECRET },
        postData: SECRET,
      },
    },
  }, source);
  journal.recordCdp({
    method: "Network.responseReceived",
    sessionId: "session-a",
    eventId: "response-1",
    params: {
      requestId: "request-1",
      type: "Fetch",
      response: {
        status: 200,
        url: `https://example.test/path?access_token=${SECRET}`,
        headers: { "set-cookie": SECRET },
        mimeType: "application/json",
        body: SECRET,
      },
    },
  }, source);
  journal.recordCdp({
    method: "Page.frameNavigated",
    sessionId: "session-a",
    eventId: "navigation-1",
    params: { frame: { loaderId: "document-b", url: `https://example.test/next?token=${SECRET}` } },
  }, source);
  currentDocument = "document-b";
  journal.recordCdp({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    eventId: "dialog-1",
    params: { type: "prompt", message: SECRET, defaultPrompt: SECRET },
  }, source);
  journal.recordCdp({
    method: "Browser.downloadWillBegin",
    eventId: "download-1",
    params: {
      frameId: "frame-a",
      guid: "download-a",
      url: `https://example.test/file?token=${SECRET}`,
      suggestedFilename: "report.txt",
    },
  }, source);
  journal.recordCdp({ method: "Chariox.browserConnected", eventId: "connected-1", params: {} }, source);

  const result = journal.poll({ cursor: 0, limit: 32, browserGeneration: 7 });
  assert.equal(result.replay_gap, false);
  assert.deepEqual(result.events.map((event) => event.kind), [
    "console",
    "network_request",
    "network_response",
    "page_navigated",
    "dialog_opened",
    "download_started",
    "browser_connected",
  ]);
  assert.deepEqual(result.events[0].actor_id, "actor-for-target-a");
  assert.equal(result.events[0].tab_id, "tab-for-target-a");
  assert.equal(result.events[0].browser_generation, 7);
  assert.deepEqual(result.events[0].data, { console_type: "error", argument_count: 4 });
  assert.equal(result.events[1].data.url, "https://example.test");
  assert.equal(result.events[2].data.url, "https://example.test");
  assert.equal(result.events[3].document_id, "document-b");
  assert.deepEqual(result.events[4].data, {
    dialog_type: "prompt",
    has_message: true,
    has_default_prompt: true,
  });
  assert.equal(result.events[5].target_id, "target-a");
  assert.equal(JSON.stringify(result).includes(SECRET), false);
  assert.equal(JSON.stringify(result).includes("authorization"), false);
  assert.equal(JSON.stringify(result).includes("cookie"), false);
  assert.equal(JSON.stringify(result).includes("postData"), false);
});

test("redacts URL userinfo/query and header values without retaining sensitive names", () => {
  assert.equal(sanitizeUrl(`https://alice:password@example.test/path?token=${SECRET}#x`), "https://example.test");
  assert.equal(
    sanitizeUrl(`https://example.test/capability/${SECRET}/reset/${SECRET}`),
    "https://example.test",
  );
  assert.equal(sanitizeUrl("data:text/plain,secret"), "data:");
  assert.equal(sanitizeUrl("about:blank?secret=1"), "about:blank");
  assert.deepEqual(redactHeaders({
    authorization: `Bearer ${SECRET}`,
    cookie: SECRET,
    "x-trace": "safe-but-not-retained",
    "x-api-token": SECRET,
  }), { "x-trace": "[redacted]" });
});

test("deduplicates reconnect replays and keeps deterministic arrival sequence IDs", () => {
  const journal = new BrowserEventJournal({ maxEvents: 8, maxBytes: 16_384 });
  const first = {
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    eventId: "source-2",
    params: { type: "log", args: [] },
  };
  const second = {
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    eventId: "source-1",
    params: { type: "warn", args: [] },
  };
  const one = journal.recordCdp(first, context());
  const two = journal.recordCdp(second, context());
  const duplicate = journal.recordCdp(first, { ...context(), reconnecting: true });
  assert.equal(one.sequence_id, 1);
  assert.equal(two.sequence_id, 2);
  assert.equal(duplicate.sequence_id, one.sequence_id);
  assert.equal(journal.size, 2);
  assert.deepEqual(journal.poll({ cursor: 0, browserGeneration: 7 }).events.map((event) => event.sequence_id), [1, 2]);
});

test("keeps identical source-less dialogs and console events instead of content-deduplicating them", () => {
  const journal = new BrowserEventJournal({ maxEvents: 8, maxBytes: 16_384 });
  const dialog = {
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "alert", message: "same" },
  };
  const consoleEvent = {
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    params: { type: "log", args: [] },
  };
  const firstDialog = journal.recordCdp(dialog, context());
  const secondDialog = journal.recordCdp(dialog, { ...context(), reconnecting: true });
  const firstConsole = journal.recordCdp(consoleEvent, context());
  const secondConsole = journal.recordCdp(consoleEvent, { ...context(), reconnecting: true });
  assert.deepEqual(
    [firstDialog.sequence_id, secondDialog.sequence_id, firstConsole.sequence_id, secondConsole.sequence_id],
    [1, 2, 3, 4],
  );
  assert.equal(journal.size, 4);
});

test("re-resolves attribution after deriving target IDs for lifecycle and download progress events", () => {
  const journal = new BrowserEventJournal({ maxEvents: 16, maxBytes: 16_384 });
  const source = {
    ...context(),
    actorId: "fallback-actor",
    tabId: "fallback-tab",
    targetIdForDownload: (guid) => (guid === "download-derived" ? "target-download" : null),
  };
  const destroyed = journal.recordCdp({
    method: "Target.targetDestroyed",
    eventId: "destroyed-1",
    params: { targetId: "target-destroyed" },
  }, source);
  const crashed = journal.recordCdp({
    method: "Target.targetCrashed",
    eventId: "crashed-1",
    params: { targetId: "target-crashed", status: "STATUS", errorCode: 7 },
  }, source);
  const progress = journal.recordCdp({
    method: "Browser.downloadProgress",
    eventId: "progress-1",
    params: { guid: "download-derived", state: "inProgress", receivedBytes: 4, totalBytes: 8 },
  }, source);
  assert.deepEqual(
    [destroyed, crashed, progress].map((event) => [event.target_id, event.actor_id, event.tab_id]),
    [
      ["target-destroyed", "actor-for-target-destroyed", "tab-for-target-destroyed"],
      ["target-crashed", "actor-for-target-crashed", "tab-for-target-crashed"],
      ["target-download", "actor-for-target-download", "tab-for-target-download"],
    ],
  );
});

test("generation changes clear stale events and reset cursors while old generation polls gap", () => {
  const journal = new BrowserEventJournal({ maxEvents: 8, maxBytes: 16_384 });
  const first = journal.recordCdp({
    method: "Chariox.browserConnected",
    eventId: "connected-1",
    params: {},
  }, context(7));
  assert.equal(first.sequence_id, 1);

  const next = journal.recordCdp({
    method: "Chariox.browserConnected",
    eventId: "connected-2",
    params: {},
  }, context(8));
  assert.equal(next.sequence_id, 1);
  assert.equal(journal.poll({ cursor: 0, browserGeneration: 7 }).replay_gap, true);
  assert.deepEqual(journal.poll({ cursor: 0, browserGeneration: 8 }).events.map((event) => event.sequence_id), [1]);
});

test("target and document invalidation drop stale events until explicitly restored", () => {
  const journal = new BrowserEventJournal({ maxEvents: 16, maxBytes: 16_384 });
  const source = context();
  const initial = journal.recordCdp({
    method: "Network.requestWillBeSent",
    sessionId: "session-a",
    eventId: "request-1",
    params: { requestId: "request-1", request: { method: "GET", url: "https://example.test/" } },
  }, source);
  assert.equal(initial.target_id, "target-a");
  assert.equal(journal.invalidateTarget({ targetId: "target-a", browserGeneration: 7 }), true);
  assert.equal(journal.isTargetInvalid("target-a"), true);
  assert.equal(journal.recordCdp({
    method: "Network.responseReceived",
    sessionId: "session-a",
    eventId: "response-stale",
    params: { requestId: "request-1", response: { status: 200, url: "https://example.test/" } },
  }, source), null);

  journal.restoreTarget("target-a");
  journal.restoreDocument("document-a");
  const restored = journal.recordCdp({
    method: "Network.responseReceived",
    sessionId: "session-a",
    eventId: "response-live",
    params: { requestId: "request-1", response: { status: 200, url: "https://example.test/" } },
  }, source);
  assert.equal(restored.kind, "network_response");
  assert.equal(journal.invalidateDocument("document-a"), true);
  assert.equal(journal.recordCdp({
    method: "Page.loadEventFired",
    sessionId: "session-a",
    eventId: "load-stale",
    params: {},
  }, source), null);
});

test("bounds lifecycle churn while retaining current target authority deterministically", () => {
  const journal = new BrowserEventJournal({
    maxEvents: 32,
    maxBytes: 32_768,
    maxLifecycleEntries: 3,
  });
  journal.record({
    kind: "page_navigated",
    eventId: "active-1",
    browserGeneration: 7,
    targetId: "active-target",
    documentId: "active-document",
    data: { url: "https://example.test/active" },
  });
  for (let index = 0; index < 10; index += 1) {
    assert.equal(journal.invalidateTarget({ targetId: `dead-target-${index}`, browserGeneration: 7 }), true);
    assert.equal(journal.invalidateDocument({ documentId: `dead-document-${index}`, browserGeneration: 7 }), true);
  }
  assert.ok(journal.invalidTargets.size <= 3);
  assert.ok(journal.invalidDocuments.size <= 3);
  assert.deepEqual([...journal.invalidTargets], ["dead-target-7", "dead-target-8", "dead-target-9"]);
  assert.deepEqual([...journal.invalidDocuments], ["dead-document-7", "dead-document-8", "dead-document-9"]);
  assert.equal(journal.targetDocuments.get("active-target"), "active-document");

  for (let index = 0; index < 8; index += 1) {
    journal.record({
      kind: "page_navigated",
      eventId: `churn-${index}`,
      browserGeneration: 7,
      targetId: `churn-target-${index}`,
      documentId: `churn-document-${index}`,
      data: { url: "https://example.test/churn" },
    });
  }
  assert.ok(journal.targetDocuments.size <= 3);
  journal.record({
    kind: "page_navigated",
    eventId: "active-2",
    browserGeneration: 7,
    targetId: "active-target",
    documentId: "active-document-2",
    data: { url: "https://example.test/active-2" },
  });
  assert.equal(journal.targetDocuments.get("active-target"), "active-document-2");
  assert.equal(journal.invalidateTarget({ targetId: "active-target", browserGeneration: 7 }), true);
  assert.equal(journal.isTargetInvalid("active-target"), true);
  assert.equal(journal.targetDocuments.has("active-target"), false);
  journal.restoreTarget("active-target");
  assert.equal(journal.isTargetInvalid("active-target"), false);
});

test("returns an oldest-first byte-bounded prefix and losslessly continues past 128 events", () => {
  const journal = new BrowserEventJournal({
    maxEvents: 256,
    maxBytes: 1_000_000,
    maxEventBytes: 4_096,
    maxSerializedBytes: 1_700,
  });
  for (let index = 0; index < 180; index += 1) {
    journal.record({
      kind: "console",
      eventId: `console-${index}`,
      browserGeneration: 7,
      targetId: "target-a",
      documentId: "document-a",
      data: { type: "log", args: [] },
    });
  }

  const blocked = journal.poll({ cursor: 0, limit: 200, browserGeneration: 7, maxSerializedBytes: 100 });
  assert.deepEqual([blocked.events.length, blocked.next_cursor, blocked.replay_gap], [0, 0, false]);

  let cursor = 0;
  const sequenceIds = [];
  for (let pageNumber = 0; pageNumber < 200; pageNumber += 1) {
    const page = journal.poll({ cursor, limit: 200, browserGeneration: 7 });
    assert.ok(byteLength(JSON.stringify(page)) <= 1_700);
    assert.equal(page.replay_gap, false);
    if (page.events.length === 0) {
      assert.equal(page.next_cursor, cursor);
      break;
    }
    assert.equal(page.events[0].sequence_id, cursor + 1);
    sequenceIds.push(...page.events.map((event) => event.sequence_id));
    cursor = page.next_cursor;
  }
  assert.deepEqual(sequenceIds, Array.from({ length: 180 }, (_, index) => index + 1));
  assert.equal(cursor, journal.cursor());

  const complete = new BrowserEventJournal({
    maxEvents: 256,
    maxBytes: 1_000_000,
    maxEventBytes: 4_096,
    maxSerializedBytes: 64 * 1_024,
  });
  for (let index = 0; index < 180; index += 1) {
    complete.record({
      kind: "console",
      eventId: `complete-${index}`,
      browserGeneration: 7,
      data: { type: "log", args: [] },
    });
  }
  assert.equal(complete.snapshot().events.length, 180);
  assert.equal(JSON.parse(complete.serialize()).events.length, 180);
});

test("evicts oldest entries first and bounds count, event bytes, and serialization", () => {
  const journal = new BrowserEventJournal({
    maxEvents: 2,
    maxBytes: 700,
    maxEventBytes: 500,
    maxSerializedBytes: 500,
  });
  const source = context();
  for (const eventId of ["one", "two", "three"]) {
    journal.recordCdp({
      method: "Runtime.consoleAPICalled",
      eventId,
      params: { type: "log", args: Array.from({ length: 200 }, () => ({ value: SECRET })) },
    }, source);
  }
  assert.ok(journal.size <= 2);
  assert.ok(journal.byteLength <= 700);
  const replay = journal.poll({ cursor: 0, browserGeneration: 7, limit: 10 });
  assert.equal(replay.replay_gap, true);
  assert.ok(byteLength(journal.serialize()) <= 500);
  assert.ok(byteLength(boundedSerialize({ payload: SECRET.repeat(1000) }, 64)) <= 64);
  assert.equal(JSON.stringify(journal.snapshot()).includes(SECRET), false);
});

test("rejects invalid limits before accepting unbounded state", () => {
  assert.throws(() => new BrowserEventJournal({ maxEvents: 0 }), (error) => {
    return error instanceof BrowserEventError && error.code === "browser_event_capacity_invalid";
  });
  assert.throws(() => new BrowserEventJournal({ maxBytes: Number.MAX_SAFE_INTEGER }), (error) => {
    return error instanceof BrowserEventError && error.code === "browser_event_bytes_invalid";
  });
  assert.throws(() => new BrowserEventJournal({ maxLifecycleEntries: 0 }), (error) => {
    return error instanceof BrowserEventError && error.code === "browser_event_lifecycle_invalid";
  });
  const journal = new BrowserEventJournal({ maxEvents: 2, maxBytes: 1024 });
  assert.throws(() => journal.poll({ cursor: 0, limit: 201 }), (error) => {
    return error instanceof BrowserEventError && error.code === "browser_event_limit_invalid";
  });
});
