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
    ...overrides,
  };
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

test("deduplicates pending and completed action IDs without rerunning", async () => {
  const coordinator = new BrowserMutationCoordinator();
  const gate = deferred();
  let calls = 0;
  const run = async () => {
    calls += 1;
    await gate.promise;
    return { ok: true };
  };
  const first = coordinator.mutate(attribution(), run);
  const duplicate = coordinator.mutate(attribution(), run);
  assert.equal(first, duplicate);
  gate.resolve();
  assert.deepEqual(await first, { ok: true });
  assert.deepEqual(await coordinator.mutate(attribution(), run), { ok: true });
  assert.equal(calls, 1);
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
