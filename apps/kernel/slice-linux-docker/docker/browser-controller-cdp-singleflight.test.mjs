import assert from "node:assert/strict";
import test, { before, after } from "node:test";

// Keep unresolved CDP waits alive so the test deadline reports a failure.
let keepAlive;
before(() => { keepAlive = setInterval(() => {}, 1000); });
after(() => clearInterval(keepAlive));

import { BrowserCdpClient } from "./browser-controller-cdp.mjs";

function deferred() {
  return Promise.withResolvers();
}

function controlledConnection(send = async () => ({})) {
  const listeners = new Set();
  const closed = deferred();
  return {
    calls: [],
    closeCount: 0,
    subscribeCount: 0,
    unsubscribeCount: 0,
    open: true,
    closed: closed.promise,
    isOpen() {
      return this.open;
    },
    async send(method, params = {}, sessionId) {
      this.calls.push({ method, params, sessionId });
      return send(method, params, sessionId);
    },
    subscribe(listener) {
      this.subscribeCount += 1;
      listeners.add(listener);
      return () => {
        this.unsubscribeCount += 1;
        listeners.delete(listener);
      };
    },
    emit(message) {
      for (const listener of listeners) listener(message);
    },
    async close() {
      if (!this.open) return;
      this.open = false;
      this.closeCount += 1;
      closed.resolve();
    },
  };
}

function countCalls(connection, method, predicate = () => true) {
  return connection.calls.filter((call) => call.method === method && predicate(call)).length;
}

test("concurrent ensureConnection calls share one connection and one subscription", { timeout: 5_000 }, async () => {
  const factoryStarted = deferred();
  const releaseFactory = deferred();
  const connection = controlledConnection();
  let factoryCalls = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      factoryCalls += 1;
      factoryStarted.resolve();
      await releaseFactory.promise;
      return connection;
    },
  });

  try {
    const first = browser.ensureConnection();
    const second = browser.ensureConnection();
    await factoryStarted.promise;
    assert.equal(factoryCalls, 1);

    releaseFactory.resolve();
    const [firstConnection, secondConnection] = await Promise.all([first, second]);
    assert.equal(firstConnection, connection);
    assert.equal(secondConnection, connection);
    assert.equal(connection.subscribeCount, 1);
    assert.equal(countCalls(connection, "Target.setDiscoverTargets"), 1);
  } finally {
    releaseFactory.resolve();
    await browser.close();
  }

  assert.equal(connection.unsubscribeCount, 1);
  assert.equal(connection.closeCount, 1);
});

test("shared connection initialization failure unsubscribes and permits retry", { timeout: 5_000 }, async () => {
  const discoveryStarted = deferred();
  const failDiscovery = deferred();
  const failedConnection = controlledConnection(async (method) => {
    if (method === "Target.setDiscoverTargets") {
      discoveryStarted.resolve();
      await failDiscovery.promise;
      throw new Error("controlled discovery failure");
    }
    return {};
  });
  const recoveredConnection = controlledConnection();
  let factoryCalls = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      factoryCalls += 1;
      return factoryCalls === 1 ? failedConnection : recoveredConnection;
    },
  });

  try {
    const first = browser.ensureConnection();
    const second = browser.ensureConnection();
    const firstFailure = assert.rejects(first, /controlled discovery failure/);
    const secondFailure = assert.rejects(second, /controlled discovery failure/);
    await discoveryStarted.promise;
    assert.equal(factoryCalls, 1);
    failDiscovery.resolve();
    await Promise.all([firstFailure, secondFailure]);

    assert.equal(failedConnection.unsubscribeCount, 1);
    assert.equal(failedConnection.closeCount, 1);
    assert.equal(await browser.ensureConnection(), recoveredConnection);
    assert.equal(factoryCalls, 2);
    assert.equal(browser.browserGeneration, 2);
  } finally {
    failDiscovery.resolve();
    await browser.close();
  }
});

test("close rejects pending connection callers and closes a late factory result without replacing a retry", { timeout: 5_000 }, async () => {
  const firstFactoryStarted = deferred();
  const releaseFirstFactory = deferred();
  const staleConnection = controlledConnection();
  const currentConnection = controlledConnection();
  let factoryCalls = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      factoryCalls += 1;
      if (factoryCalls === 1) {
        firstFactoryStarted.resolve();
        await releaseFirstFactory.promise;
        return staleConnection;
      }
      return currentConnection;
    },
  });

  try {
    const pending = browser.ensureConnection();
    const invalidated = assert.rejects(pending, { code: "browser_cdp_disconnected" });
    await firstFactoryStarted.promise;
    await browser.close();
    await invalidated;

    assert.equal(await browser.ensureConnection(), currentConnection);
    releaseFirstFactory.resolve();
    await staleConnection.closed;
    assert.equal(browser.connection, currentConnection);
    assert.equal(staleConnection.subscribeCount, 0);
    assert.equal(currentConnection.subscribeCount, 1);
    assert.equal(browser.browserGeneration, 1);
  } finally {
    releaseFirstFactory.resolve();
    await browser.close();
  }
});

test("concurrent target-session calls share one attach and one setup", { timeout: 5_000 }, async () => {
  const attachStarted = deferred();
  const releaseAttach = deferred();
  const connection = controlledConnection(async (method) => {
    if (method === "Target.attachToTarget") {
      attachStarted.resolve();
      await releaseAttach.promise;
      return { sessionId: "session-a" };
    }
    return {};
  });
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });

  try {
    const activeConnection = await browser.ensureConnection();
    const first = browser.ensureTargetSession(activeConnection, "target-a");
    await attachStarted.promise;
    const second = browser.ensureTargetSession(activeConnection, "target-a");
    assert.equal(countCalls(connection, "Target.attachToTarget"), 1);

    releaseAttach.resolve();
    const [firstSession, secondSession] = await Promise.all([first, second]);
    assert.equal(firstSession, "session-a");
    assert.equal(secondSession, "session-a");
    assert.equal(browser.sessionsByTarget.get("target-a"), "session-a");
    for (const method of [
      "Page.enable",
      "Page.setLifecycleEventsEnabled",
      "Runtime.enable",
      "Network.enable",
      "Inspector.enable",
      "Target.setAutoAttach",
    ]) {
      assert.equal(countCalls(connection, method), 1, `${method} should run once`);
    }
  } finally {
    releaseAttach.resolve();
    await browser.close();
  }
});

test("failed target setup rejects all waiters, detaches once, and permits retry", { timeout: 5_000 }, async () => {
  const pageEnableStarted = deferred();
  const failPageEnable = deferred();
  let attachCalls = 0;
  const connection = controlledConnection(async (method, _params, sessionId) => {
    if (method === "Target.attachToTarget") {
      attachCalls += 1;
      return { sessionId: `session-${attachCalls}` };
    }
    if (method === "Page.enable" && sessionId === "session-1") {
      pageEnableStarted.resolve();
      await failPageEnable.promise;
      throw new Error("controlled page setup failure");
    }
    return {};
  });
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });

  try {
    const activeConnection = await browser.ensureConnection();
    const first = browser.ensureTargetSession(activeConnection, "target-a");
    const firstFailure = assert.rejects(first, /controlled page setup failure/);
    await pageEnableStarted.promise;
    const second = browser.ensureTargetSession(activeConnection, "target-a");
    const secondFailure = assert.rejects(second, /controlled page setup failure/);
    assert.equal(attachCalls, 1);

    failPageEnable.resolve();
    await Promise.all([firstFailure, secondFailure]);
    assert.equal(browser.sessionsByTarget.has("target-a"), false);
    assert.equal(countCalls(connection, "Target.detachFromTarget"), 1);

    assert.equal(await browser.ensureTargetSession(activeConnection, "target-a"), "session-2");
    assert.equal(attachCalls, 2);
    assert.equal(browser.sessionsByTarget.get("target-a"), "session-2");
  } finally {
    failPageEnable.resolve();
    await browser.close();
  }
});

test("close during target setup rejects waiters and stale cleanup cannot erase a new session", { timeout: 5_000 }, async () => {
  const pageEnableStarted = deferred();
  const releasePageEnable = deferred();
  const staleDetach = deferred();
  const staleConnection = controlledConnection(async (method, params, sessionId) => {
    if (method === "Target.attachToTarget") return { sessionId: "stale-session" };
    if (method === "Page.enable") {
      pageEnableStarted.resolve();
      await releasePageEnable.promise;
    }
    if (method === "Target.detachFromTarget" && params.sessionId === "stale-session") {
      staleDetach.resolve();
    }
    return {};
  });
  const recoveredConnection = controlledConnection(async (method) => {
    if (method === "Target.attachToTarget") return { sessionId: "current-session" };
    return {};
  });
  let factoryCalls = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => (++factoryCalls === 1 ? staleConnection : recoveredConnection),
  });

  try {
    const oldConnection = await browser.ensureConnection();
    const pending = browser.ensureTargetSession(oldConnection, "target-a");
    const invalidated = assert.rejects(pending, { code: "browser_cdp_disconnected" });
    await pageEnableStarted.promise;
    await browser.close();
    await invalidated;

    const newConnection = await browser.ensureConnection();
    assert.equal(await browser.ensureTargetSession(newConnection, "target-a"), "current-session");
    releasePageEnable.resolve();
    await staleDetach.promise;
    assert.equal(browser.sessionsByTarget.get("target-a"), "current-session");
    assert.equal(browser.targetsBySession.get("current-session"), "target-a");
  } finally {
    releasePageEnable.resolve();
    await browser.close();
  }
});

test("target detachment before the attach reply rejects that session and permits retry", { timeout: 5_000 }, async () => {
  const firstAttachStarted = deferred();
  const releaseFirstAttach = deferred();
  const staleDetach = deferred();
  let attachCalls = 0;
  const connection = controlledConnection(async (method, params) => {
    if (method === "Target.attachToTarget") {
      attachCalls += 1;
      if (attachCalls === 1) {
        firstAttachStarted.resolve();
        await releaseFirstAttach.promise;
        return { sessionId: "stale-session" };
      }
      return { sessionId: "current-session" };
    }
    if (method === "Target.detachFromTarget" && params.sessionId === "stale-session") {
      staleDetach.resolve();
    }
    return {};
  });
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });

  try {
    const activeConnection = await browser.ensureConnection();
    const pending = browser.ensureTargetSession(activeConnection, "target-a");
    const invalidated = assert.rejects(pending, { code: "browser_cdp_disconnected" });
    await firstAttachStarted.promise;
    connection.emit({
      method: "Target.detachedFromTarget",
      sessionId: "stale-session",
      params: { sessionId: "stale-session", targetId: "target-a" },
    });
    releaseFirstAttach.resolve();
    await invalidated;
    await staleDetach.promise;

    assert.equal(await browser.ensureTargetSession(activeConnection, "target-a"), "current-session");
    assert.equal(attachCalls, 2);
    assert.equal(browser.sessionsByTarget.get("target-a"), "current-session");
  } finally {
    releaseFirstAttach.resolve();
    await browser.close();
  }
});

for (const writer of [false, true]) {
  const kind = writer ? "writer" : "page";
  test(`MP-08/MP-10/MP-11 concurrent ${kind} setup waits for every domain before detaching a failed session`, { timeout: 5_000 }, async () => {
    const started = deferred(), release = deferred();
    let attachCalls = 0;
    const connection = controlledConnection(async method => {
      if (method === "Target.attachToTarget") return { sessionId: `session-${++attachCalls}` };
      if (attachCalls === 1 && method === "Network.enable") throw new Error("network setup refused");
      if (attachCalls === 1 && method === (writer ? "Debugger.enable" : "Runtime.enable")) {
        started.resolve();
        await release.promise;
      }
      return {};
    });
    const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
    const ensure = writer ? browser.ensureWriterTargetSession.bind(browser) : browser.ensureTargetSession.bind(browser);
    try {
      await browser.ensureConnection();
      const first = ensure(connection, "target-a");
      const second = ensure(connection, "target-a");
      const rejected = Promise.all([first, second].map(promise => assert.rejects(promise, /network setup refused/)));
      await started.promise;
      await new Promise(resolve => setImmediate(resolve));
      assert.equal(countCalls(connection, "Target.detachFromTarget"), 0);
      release.resolve();
      await rejected;
      assert.equal(attachCalls, 1);
      assert.equal(countCalls(connection, "Target.detachFromTarget"), 1);
      assert.equal(await ensure(connection, "target-a"), "session-2");
    } finally {
      release.resolve();
      await browser.close();
    }
  });

  for (const detachBeforeReply of [false, true]) {
    test(`MP-08/MP-09/MP-10/MP-11 retired ${kind} detach ${detachBeforeReply ? "before" : "after"} its attach reply preserves a pending replacement`, { timeout: 5_000 }, async () => {
      const firstStarted = deferred(), releaseFirst = deferred(), firstDetached = deferred();
      const replacementStarted = deferred(), releaseReplacement = deferred();
      let attachCalls = 0;
      const detach = () => connection.emit({
        method: "Target.detachedFromTarget",
        params: { sessionId: "retired", targetId: "target-a" },
      });
      const connection = controlledConnection(async (method, params) => {
        if (method === "Target.attachToTarget") {
          if (++attachCalls === 1) {
            firstStarted.resolve();
            await releaseFirst.promise;
            return { sessionId: "retired" };
          }
          replacementStarted.resolve();
          await releaseReplacement.promise;
          return { sessionId: "replacement" };
        }
        if (method === "Target.detachFromTarget" && params.sessionId === "retired") {
          if (!detachBeforeReply) detach();
          firstDetached.resolve();
        }
        return {};
      });
      const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
      const ensure = writer ? browser.ensureWriterTargetSession.bind(browser) : browser.ensureTargetSession.bind(browser);
      try {
        await browser.ensureConnection();
        const first = ensure(connection, "target-a");
        const rejected = assert.rejects(first, { code: "browser_cdp_disconnected" });
        await firstStarted.promise;
        await browser.forgetTarget("target-a");
        await rejected;

        const replacements = [ensure(connection, "target-a"), ensure(connection, "target-a")];
        const results = replacements.map(promise => promise.then(sessionId => ({ sessionId }), error => ({ code: error.code })));
        await replacementStarted.promise;
        if (detachBeforeReply) detach();
        releaseFirst.resolve();
        await firstDetached.promise;
        releaseReplacement.resolve();
        assert.deepEqual(await Promise.all(results), [{ sessionId: "replacement" }, { sessionId: "replacement" }]);
        assert.equal(await ensure(connection, "target-a"), "replacement");
        assert.equal(attachCalls, 2);
        assert.equal(browser.targetsBySession.get("replacement"), "target-a");
        assert.equal(countCalls(connection, "Target.detachFromTarget", call => call.params.sessionId === "replacement"), 0);
      } finally {
        releaseFirst.resolve();
        releaseReplacement.resolve();
        await browser.close();
      }
    });
  }

  test(`MP-08/MP-09/MP-10/MP-11 current ${kind} detach before its attach reply rejects callers and allows retry`, { timeout: 5_000 }, async () => {
    const started = deferred(), release = deferred();
    let attachCalls = 0;
    const connection = controlledConnection(async method => {
      if (method === "Target.attachToTarget") {
        if (++attachCalls === 1) { started.resolve(); await release.promise; return { sessionId: "detached" }; }
        return { sessionId: "replacement" };
      }
      return {};
    });
    const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
    const ensure = writer ? browser.ensureWriterTargetSession.bind(browser) : browser.ensureTargetSession.bind(browser);
    try {
      await browser.ensureConnection();
      const first = ensure(connection, "target-a"), second = ensure(connection, "target-a");
      const rejected = Promise.all([first, second].map(promise => assert.rejects(promise, { code: "browser_cdp_disconnected" })));
      await started.promise;
      connection.emit({ method: "Target.detachedFromTarget", params: { sessionId: "detached", targetId: "target-a" } });
      release.resolve();
      await rejected;
      assert.equal(countCalls(connection, "Page.enable"), 0);
      assert.equal(countCalls(connection, "Network.enable"), 0);
      assert.equal(await ensure(connection, "target-a"), "replacement");
    } finally {
      release.resolve();
      await browser.close();
    }
  });

  test(`MP-08/MP-10/MP-11 forgetting a ${kind} target rejects all attaching callers before a late reply`, { timeout: 5_000 }, async () => {
    const started = deferred(), release = deferred(), detached = deferred();
    let attachCalls = 0;
    const connection = controlledConnection(async (method, params) => {
      if (method === "Target.attachToTarget") {
        if (++attachCalls === 1) { started.resolve(); await release.promise; return { sessionId: "stale" }; }
        return { sessionId: "current" };
      }
      if (method === "Target.detachFromTarget" && params.sessionId === "stale") detached.resolve();
      return {};
    });
    const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
    const ensure = writer ? browser.ensureWriterTargetSession.bind(browser) : browser.ensureTargetSession.bind(browser);
    try {
      await browser.ensureConnection();
      const first = ensure(connection, "target-a");
      const second = ensure(connection, "target-a");
      const rejected = Promise.all([first, second].map(promise => assert.rejects(promise, { code: "browser_cdp_disconnected" })));
      await started.promise;
      await browser.forgetTarget("target-a");
      await rejected;
      assert.equal(await ensure(connection, "target-a"), "current");
      release.resolve();
      await detached.promise;
      assert.equal(browser.sessionsByTarget.get("target-a"), "current");
    } finally {
      release.resolve();
      await browser.close();
    }
  });
}

test("MP-08/MP-10/MP-11 disconnect events still reach the browser journal after the socket closes", async () => {
  const connection = controlledConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  try {
    await browser.ensureConnection();
    await connection.close();
    connection.emit({ method: "Chariox.browserDisconnected", params: { code: "browser_cdp_disconnected" } });
    assert(browser.eventJournal.events.some(event => event.kind === "browser_disconnected"));
  } finally {
    await browser.close();
  }
});
