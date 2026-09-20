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
  assert.equal(result.events[1].data.url, "https://example.test/path");
  assert.equal(result.events[2].data.url, "https://example.test/path");
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
  assert.equal(sanitizeUrl(`https://alice:password@example.test/path?token=${SECRET}#x`), "https://example.test/path");
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
  const journal = new BrowserEventJournal({ maxEvents: 2, maxBytes: 1024 });
  assert.throws(() => journal.poll({ cursor: 0, limit: 201 }), (error) => {
    return error instanceof BrowserEventError && error.code === "browser_event_limit_invalid";
  });
});
