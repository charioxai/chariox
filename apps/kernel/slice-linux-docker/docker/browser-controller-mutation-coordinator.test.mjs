import { createHash } from "node:crypto";
import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserMutationCoordinator,
  MUTATION_ERROR_CODES,
} from "./browser-controller-mutation-coordinator.mjs";

function attribution(overrides = {}) {
  return {
    action_id: "action-1",
    actor_id: "agent-1",
    browser_generation: 1,
    operation: "click",
    tab_id: "tab-1",
    target_generation: 1,
    target_id: "target-1",
    page_id: "page-1",
    document_id: "document-1",
    arguments: { element_ref: "element-1" },
    payload: { value: "first" },
    ...overrides,
  };
}

function bareAttribution(overrides = {}) {
  return {
    action_id: "action-1",
    actor_id: "agent-1",
    browser_generation: 1,
    operation: "click",
    tab_id: "tab-1",
    target_generation: 1,
    ...overrides,
  };
}

function mutationIdentity(overrides = {}) {
  return {
    target_id: "target-1",
    page_id: "page-1",
    document_id: "document-1",
    arguments: { element_ref: "element-1" },
    payload: { value: "first" },
    ...overrides,
  };
}

function legacyPayloadDigest(rawAttribution, identity) {
  const semanticIdentity = {
    action_id: rawAttribution.action_id,
    actor_id: rawAttribution.actor_id,
    browser_generation: rawAttribution.browser_generation,
    operation: rawAttribution.operation,
    tab_id: rawAttribution.tab_id,
    target_generation: rawAttribution.target_generation,
    target_id: identity.target_id,
    page_id: identity.page_id,
    document_id: identity.document_id,
    arguments: identity.arguments,
    payload: identity.payload,
  };
  return createHash("sha256").update(JSON.stringify(semanticIdentity)).digest("hex");
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, reject, resolve };
}

function assertTerminalRecord(record, outcome, secret) {
  assert.deepEqual(Object.keys(record).sort(), [
    "fingerprint",
    "outcome",
    "promise",
  ]);
  assert.equal(record.outcome, outcome);
  assert.equal(typeof record.promise?.then, "function");
  assert.equal(record.fingerprint.includes(secret), false);
  assert.equal(Object.isFrozen(record), true);
  for (const key of ["abortController", "attribution", "reject", "resolve", "run", "request", "payload"]) {
    assert.equal(key in record, false, `terminal record retained ${key}`);
  }
}

test("serializes mutations on one tab and preserves actor attribution", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const first = deferred();
  const order = [];
  const firstPromise = coordinator.mutate(attribution(), async ({ attribution: actor }) => {
    order.push(`start:${actor.actor_id}`);
    await first.promise;
    order.push("end:first");
    return "first";
  });
  const secondPromise = coordinator.mutate(
    attribution({ action_id: "action-2", actor_id: "human-1", operation: "fill" }),
    async ({ attribution: actor }) => {
      order.push(`start:${actor.actor_id}`);
      return "second";
    },
  );
  await Promise.resolve();
  assert.deepEqual(order, ["start:agent-1"]);
  first.resolve();
  assert.deepEqual(await Promise.all([firstPromise, secondPromise]), ["first", "second"]);
  assert.deepEqual(order, ["start:agent-1", "end:first", "start:human-1"]);
});
test("allows independent tabs to mutate concurrently", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const first = deferred();
  const second = deferred();
  let active = 0;
  let peak = 0;
  const run = (gate) => async () => {
    active += 1;
    peak = Math.max(peak, active);
    await gate.promise;
    active -= 1;
  };
  const left = coordinator.mutate(attribution(), run(first));
  const right = coordinator.mutate(
    attribution({ action_id: "action-2", tab_id: "tab-2" }),
    run(second),
  );
  await Promise.resolve();
  assert.equal(peak, 2);
  first.resolve();
  second.resolve();
  await Promise.all([left, right]);
});

test("rejects mutation without a canonical identity or opaque request fingerprint", () => {
  const coordinator = new BrowserMutationCoordinator();
  assert.throws(
    () => coordinator.mutate(bareAttribution(), async () => "must-not-run"),
    (error) => error.code === MUTATION_ERROR_CODES.INVALID_ARGUMENT,
  );
  assert.throws(
    () => coordinator.mutate(
      bareAttribution({ action_id: "partial-identity" }),
      async () => "must-not-run",
      { payload: { value: "partial" } },
    ),
    (error) => error.code === MUTATION_ERROR_CODES.INVALID_ARGUMENT,
  );
});

test("requires identity for payload-free operations and deduplicates opaque fingerprints", async () => {
  const coordinator = new BrowserMutationCoordinator();
  let canonicalCalls = 0;
  const payloadFreeAttribution = attribution({ action_id: "payload-free" });
  const payloadFreeIdentity = mutationIdentity({ arguments: null, payload: null });
  const canonical = coordinator.mutate(
    payloadFreeAttribution,
    async () => {
      canonicalCalls += 1;
      return "focused";
    },
    payloadFreeIdentity,
  );
  assert.equal(
    coordinator.mutate(payloadFreeAttribution, async () => "must-not-run", payloadFreeIdentity),
    canonical,
  );
  assert.equal(await canonical, "focused");
  assert.equal(canonicalCalls, 1);

  let opaqueCalls = 0;
  const opaqueAttribution = bareAttribution({
    action_id: "opaque-payload-free",
    operation: "focus",
  });
  const opaqueIdentity = { request_fingerprint: "focus-target-v1" };
  const opaque = coordinator.mutate(
    opaqueAttribution,
    async () => {
      opaqueCalls += 1;
      return "opaque-focused";
    },
    opaqueIdentity,
  );
  assert.equal(
    coordinator.mutate(opaqueAttribution, async () => "must-not-run", opaqueIdentity),
    opaque,
  );
  assert.equal(await opaque, "opaque-focused");
  assert.equal(opaqueCalls, 1);
});

test("rejects changed target and text under one action ID", async () => {
  const coordinator = new BrowserMutationCoordinator();
  let calls = 0;
  const fillAttribution = bareAttribution({
    action_id: "fill-once",
    operation: "fill",
  });
  const first = coordinator.mutate(
    fillAttribution,
    async () => {
      calls += 1;
      return "filled-first-value";
    },
    mutationIdentity({ target_id: "target-first", payload: { text: "first" } }),
  );
  assert.throws(
    () => coordinator.mutate(
      fillAttribution,
      async () => "must-not-run",
      mutationIdentity({ target_id: "target-second", payload: { text: "second" } }),
    ),
    (error) => error.code === MUTATION_ERROR_CODES.ACTION_ID_CONFLICT,
  );
  assert.equal(await first, "filled-first-value");
  assert.equal(calls, 1);
});

test("deduplicates pending and completed action IDs without rerunning", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const gate = deferred();
  let calls = 0;
  const identity = mutationIdentity();
  const run = async () => {
    calls += 1;
    await gate.promise;
    return { ok: true };
  };
  const first = coordinator.mutate(attribution(), run, identity);
  const duplicate = coordinator.mutate(attribution(), run, identity);
  assert.equal(first, duplicate);
  gate.resolve();
  assert.deepEqual(await first, { ok: true });
  assert.deepEqual(await coordinator.mutate(attribution(), run, identity), { ok: true });
  assert.equal(calls, 1);
});

test("does not deduplicate one action ID across target, page, or document", async () => {
  for (const field of ["target_id", "page_id", "document_id"]) {
    const coordinator = new BrowserMutationCoordinator();
    const first = coordinator.mutate(
      attribution(),
      async () => "first",
      mutationIdentity({ [field]: `${field}-other` }),
    );
    assert.throws(
      () => coordinator.mutate(
        attribution(),
        async () => "must-not-run",
        mutationIdentity({ [field]: `${field}-different` }),
      ),
      (error) => error.code === MUTATION_ERROR_CODES.ACTION_ID_CONFLICT,
    );
    assert.equal(await first, "first");
  }
});

test("does not deduplicate one action ID across different mutation payloads", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const first = coordinator.mutate(
    attribution(),
    async () => "first",
    mutationIdentity({ payload: { value: "first" } }),
  );
  assert.equal(await first, "first");
  assert.throws(
    () => coordinator.mutate(
      attribution(),
      async () => "must-not-run",
      mutationIdentity({ payload: { value: "different" } }),
    ),
    (error) => error.code === MUTATION_ERROR_CODES.ACTION_ID_CONFLICT,
  );
});

test("deduplicates key-order-equivalent normalized arguments and payloads", async () => {
  const coordinator = new BrowserMutationCoordinator();
  let calls = 0;
  const first = coordinator.mutate(
    attribution(),
    async () => {
      calls += 1;
      return "same-result";
    },
    mutationIdentity({
      arguments: { z: 1, nested: { b: true, a: "same" }, a: [2, 1] },
      payload: { second: "value", first: { b: 2, a: 1 } },
    }),
  );
  const retry = coordinator.mutate(
    attribution(),
    async () => "must-not-run",
    mutationIdentity({
      arguments: { a: [2, 1], nested: { a: "same", b: true }, z: 1 },
      payload: { first: { a: 1, b: 2 }, second: "value" },
    }),
  );
  assert.equal(retry, first);
  assert.equal(await retry, "same-result");
  assert.equal(calls, 1);
});

test("deduplicates a completed action retry after browser generation advance", async () => {
  const coordinator = new BrowserMutationCoordinator({ browserGeneration: 1 });
  const original = attribution({ action_id: "generation-retained" });
  let calls = 0;
  const first = coordinator.mutate(original, async () => {
    calls += 1;
    return "completed";
  });
  assert.equal(await first, "completed");

  coordinator.advanceBrowserGeneration(2);
  const retry = coordinator.mutate(original, async () => {
    calls += 1;
    return "must-not-replay";
  });
  assert.equal(retry, first);
  assert.equal(await retry, "completed");
  assert.equal(calls, 1);

  assert.throws(
    () => coordinator.mutate(
      attribution({
        action_id: original.action_id,
        browser_generation: 2,
        operation: "fill",
      }),
      async () => "must-conflict",
    ),
    (error) => error.code === MUTATION_ERROR_CODES.ACTION_ID_CONFLICT,
  );
});

test("retains only bounded secret-safe terminal state after completion", async () => {
  const coordinator = new BrowserMutationCoordinator({ maxCompleted: 2 });
  const secret = "Bearer completion-secret";
  const payload = { authorization: secret };
  const firstAttribution = attribution({ action_id: "completed-1" });
  await coordinator.mutate(
    firstAttribution,
    async () => payload.authorization && "one",
    mutationIdentity({ payload }),
  );
  await coordinator.mutate(attribution({ action_id: "completed-2" }), async () => "two");
  const thirdAttribution = attribution({ action_id: "completed-3" });
  const third = coordinator.mutate(
    thirdAttribution,
    async () => "three",
    mutationIdentity({ payload }),
  );
  assert.equal(await third, "three");

  assert.equal(coordinator.actions.has(firstAttribution.action_id), false);
  assert.equal(coordinator.snapshot().completed_count, 2);
  const retained = coordinator.actions.get(thirdAttribution.action_id);
  assertTerminalRecord(retained, "fulfilled", secret);
  assert.equal(
    coordinator.mutate(thirdAttribution, async () => "must-not-run", mutationIdentity({ payload })),
    retained.promise,
  );
});

test("retained fingerprints do not enable a candidate dictionary recovery", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const completedAttribution = attribution({
    action_id: "secret-completed",
    operation: "fill",
  });
  const completedIdentity = mutationIdentity({ payload: { authorization: "1234" } });
  await coordinator.mutate(completedAttribution, async () => "completed", completedIdentity);

  const retained = coordinator.actions.get(completedAttribution.action_id);
  const retainedFingerprint = JSON.parse(retained.fingerprint);
  assert.match(retainedFingerprint.semantic_digest, /^[0-9a-f]{64}$/u);
  assert.equal(JSON.stringify(retained).includes("1234"), false);
  const candidates = ["0000", "1234", "2468", "9999"];
  assert.equal(
    candidates.some((candidate) => (
      legacyPayloadDigest(
        completedAttribution,
        mutationIdentity({ payload: { authorization: candidate } }),
      ) === retainedFingerprint.semantic_digest
    )),
    false,
  );
});

test("rejects an action ID reused with different attribution", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const gate = deferred();
  const current = coordinator.mutate(attribution(), () => gate.promise);
  assert.throws(
    () => coordinator.mutate(attribution({ actor_id: "human-1" }), async () => {}),
    (error) => error.code === MUTATION_ERROR_CODES.ACTION_ID_CONFLICT,
  );
  gate.resolve();
  await current;
});

test("cancels queued mutations and marks active work indeterminate on tab invalidation", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const gate = deferred();
  let activeSignal;
  const active = coordinator.mutate(attribution(), async ({ signal }) => {
    activeSignal = signal;
    await gate.promise;
    return "mutated";
  });
  const queued = coordinator.mutate(
    attribution({ action_id: "action-2" }),
    async () => "must-not-run",
  );
  await Promise.resolve();
  coordinator.invalidateTab("tab-1", 1);
  assert.equal(activeSignal.aborted, true);
  await assert.rejects(queued, (error) => error.code === MUTATION_ERROR_CODES.CANCELLED);
  gate.resolve();
  await assert.rejects(active, (error) => error.code === MUTATION_ERROR_CODES.INDETERMINATE);
  assert.throws(
    () => coordinator.mutate(attribution({ action_id: "action-3" }), async () => {}),
    (error) => error.code === MUTATION_ERROR_CODES.TAB_GENERATION_STALE,
  );
});

test("invalidates only older active and queued generations and preserves retry", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const activeGate = deferred();
  let activeSignal;
  let newerCalls = 0;
  const active = coordinator.mutate(
    attribution({ action_id: "older-active", target_generation: 1 }),
    async ({ signal }) => {
      activeSignal = signal;
      await activeGate.promise;
      return "old";
    },
  );
  const newerAttribution = attribution({
    action_id: "newer-queued",
    operation: "fill",
    target_generation: 2,
  });
  const newer = coordinator.mutate(newerAttribution, async () => {
    newerCalls += 1;
    return "new";
  });
  await Promise.resolve();

  coordinator.invalidateTab("tab-1", 1);
  assert.equal(activeSignal.aborted, true);
  assert.deepEqual(coordinator.snapshot().queued, [
    { tab_id: "tab-1", action_ids: ["newer-queued"] },
  ]);

  const retry = coordinator.mutate(newerAttribution, async () => "must-not-replay");
  assert.equal(retry, newer);
  activeGate.resolve();
  await assert.rejects(active, (error) => error.code === MUTATION_ERROR_CODES.INDETERMINATE);
  assert.equal(await retry, "new");
  assert.equal(newerCalls, 1);
});

test("releases execution references for queued cancellation", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const activeGate = deferred();
  const secret = "Bearer queued-secret";
  const payload = { authorization: secret };
  const active = coordinator.mutate(
    attribution({ action_id: "newer-active", target_generation: 2 }),
    () => activeGate.promise,
  );
  const queuedAttribution = attribution({
    action_id: "older-queued",
    operation: "fill",
    target_generation: 1,
  });
  const queued = coordinator.mutate(
    queuedAttribution,
    async () => payload.authorization && "must-not-run",
    mutationIdentity({ payload }),
  );
  await Promise.resolve();

  coordinator.invalidateTab("tab-1", 1);
  await assert.rejects(queued, (error) => error.code === MUTATION_ERROR_CODES.CANCELLED);
  const retained = coordinator.actions.get(queuedAttribution.action_id);
  assertTerminalRecord(retained, "rejected", secret);
  assert.equal(retained.promise, queued);

  activeGate.resolve("active");
  assert.equal(await active, "active");
});

test("does not start an admitted mutation after its tab is invalidated", async () => {
  const coordinator = new BrowserMutationCoordinator();
  let started = false;
  const result = coordinator.mutate(attribution(), async () => {
    started = true;
    return "must-not-run";
  });

  coordinator.invalidateTab("tab-1", 1);

  await assert.rejects(
    result,
    (error) => error.code === MUTATION_ERROR_CODES.INDETERMINATE,
  );
  assert.equal(started, false);
});

test("browser generation advance invalidates old authority", async () => {
  const coordinator = new BrowserMutationCoordinator({ browserGeneration: 1 });
  const gate = deferred();
  const active = coordinator.mutate(attribution(), async () => gate.promise);
  await Promise.resolve();
  coordinator.advanceBrowserGeneration(2);
  gate.resolve();
  await assert.rejects(active, (error) => error.code === MUTATION_ERROR_CODES.INDETERMINATE);
  assert.throws(
    () => coordinator.mutate(attribution({ action_id: "old" }), async () => {}),
    (error) => error.code === MUTATION_ERROR_CODES.BROWSER_GENERATION_STALE,
  );
  assert.equal(
    await coordinator.mutate(
      attribution({ action_id: "new", browser_generation: 2 }),
      async () => "ok",
    ),
    "ok",
  );
});

test("bounds invalidation tombstones and fails closed until browser generation advances", async () => {
  const coordinator = new BrowserMutationCoordinator({ maxInvalidatedTabs: 2 });
  coordinator.invalidateTab("tab-1", 1);
  coordinator.invalidateTab("tab-2", 2);
  coordinator.invalidateTab("tab-3", 3);

  assert.deepEqual(
    {
      browser_generation: coordinator.snapshot().browser_generation,
      invalidated_tab_count: coordinator.snapshot().invalidated_tab_count,
      invalidation_overflowed: coordinator.snapshot().invalidation_overflowed,
    },
    {
      browser_generation: null,
      invalidated_tab_count: 2,
      invalidation_overflowed: true,
    },
  );
  assert.throws(
    () => coordinator.mutate(attribution(), async () => "must-not-run"),
    (error) => error.code === MUTATION_ERROR_CODES.QUEUE_SATURATED,
  );

  coordinator.advanceBrowserGeneration(1);
  assert.equal(
    await coordinator.mutate(attribution({ action_id: "after-reset" }), async () => "ok"),
    "ok",
  );
  assert.equal(coordinator.snapshot().invalidated_tab_count, 0);
  assert.equal(coordinator.snapshot().invalidation_overflowed, false);
});

test("deduplicates a pending action during invalidation overflow", async () => {
  const coordinator = new BrowserMutationCoordinator({ maxInvalidatedTabs: 1 });
  const gate = deferred();
  const original = attribution({ action_id: "overflow-pending", tab_id: "tab-pending" });
  const pending = coordinator.mutate(original, async () => {
    await gate.promise;
    return "pending-result";
  });
  await Promise.resolve();

  coordinator.invalidateTab("tab-overflow-1", 1);
  coordinator.invalidateTab("tab-overflow-2", 1);
  assert.equal(coordinator.snapshot().invalidation_overflowed, true);

  const retry = coordinator.mutate(original, async () => "must-not-replay");
  assert.equal(retry, pending);
  assert.throws(
    () => coordinator.mutate(
      attribution({
        action_id: original.action_id,
        operation: "fill",
        tab_id: original.tab_id,
      }),
      async () => "must-conflict",
    ),
    (error) => error.code === MUTATION_ERROR_CODES.ACTION_ID_CONFLICT,
  );

  gate.resolve();
  assert.equal(await retry, "pending-result");
});

test("preserves undefined rejection for the original action and deduplicated replay", async () => {
  const coordinator = new BrowserMutationCoordinator();
  let calls = 0;
  const observe = (promise) => promise.then(
    (value) => ({ status: "fulfilled", value }),
    (reason) => ({ status: "rejected", reason }),
  );

  const first = await observe(coordinator.mutate(attribution(), async () => {
    calls += 1;
    return Promise.reject();
  }));
  const replay = await observe(coordinator.mutate(attribution(), async () => {
    calls += 1;
    return "must-not-run";
  }));

  assert.deepEqual(first, { status: "rejected", reason: undefined });
  assert.deepEqual(replay, { status: "rejected", reason: undefined });
  assert.equal(calls, 1);
});

test("bounds queued tabs, per-tab work, and completed deduplication", async () => {
  const coordinator = new BrowserMutationCoordinator({
    maxCompleted: 1,
    maxQueuedPerTab: 1,
    maxTabs: 1,
  });
  const gate = deferred();
  const first = coordinator.mutate(attribution(), () => gate.promise);
  const second = coordinator.mutate(attribution({ action_id: "action-2" }), async () => "two");
  assert.throws(
    () => coordinator.mutate(attribution({ action_id: "action-3" }), async () => "three"),
    (error) => error.code === MUTATION_ERROR_CODES.QUEUE_SATURATED,
  );
  assert.throws(
    () => coordinator.mutate(attribution({ action_id: "other", tab_id: "tab-2" }), async () => {}),
    (error) => error.code === MUTATION_ERROR_CODES.QUEUE_SATURATED,
  );
  gate.resolve("one");
  assert.deepEqual(await Promise.all([first, second]), ["one", "two"]);
  assert.equal(coordinator.snapshot().completed_count, 1);
});

test("snapshot is deterministic and contains attribution but no mutation payload", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const gate = deferred();
  const right = coordinator.mutate(
    attribution({ action_id: "right", actor_id: "human-1", tab_id: "tab-2" }),
    () => gate.promise,
  );
  const left = coordinator.mutate(
    attribution({ action_id: "left", actor_id: "agent-2", tab_id: "tab-1" }),
    () => gate.promise,
  );
  await Promise.resolve();
  assert.deepEqual(
    coordinator.snapshot().active.map(({ tab_id, actor_id }) => ({ tab_id, actor_id })),
    [
      { tab_id: "tab-1", actor_id: "agent-2" },
      { tab_id: "tab-2", actor_id: "human-1" },
    ],
  );
  assert.equal(JSON.stringify(coordinator.snapshot()).includes("password"), false);
  gate.resolve();
  await Promise.all([left, right]);
});

test("validates exact attribution schema", () => {
  const coordinator = new BrowserMutationCoordinator();
  for (const value of [
    {},
    attribution({ extra: true }),
    attribution({ actor_id: "actor with spaces" }),
    attribution({ operation: "Click" }),
    attribution({ browser_generation: 0 }),
  ]) {
    assert.throws(
      () => coordinator.mutate(value, async () => {}),
      (error) => error.code === MUTATION_ERROR_CODES.INVALID_ARGUMENT,
    );
  }
});
