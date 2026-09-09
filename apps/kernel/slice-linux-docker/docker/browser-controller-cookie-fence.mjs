const WRITER_TARGET_FILTER = [
  { type: "page" },
  { type: "worker" },
  { type: "shared_worker" },
  { type: "service_worker" },
  { exclude: true },
];

const OFFLINE = {
  offline: true,
  latency: 0,
  downloadThroughput: -1,
  uploadThroughput: -1,
};

const ONLINE = { ...OFFLINE, offline: false };

export async function acquireBrowserCookieWriterFence({ connection, pageSessions, waitForNetworkIdle }) {
  const pausedSessions = new Set();
  const frozenSessions = [];
  const unsubscribe = connection.subscribe((message) => {
    const { sessionId, targetInfo, waitingForDebugger } = message?.params ?? {};
    if (message?.method === "Target.attachedToTarget"
        && waitingForDebugger === true
        && WRITER_TARGET_FILTER.some((entry) => !entry.exclude && entry.type === targetInfo?.type)
        && typeof sessionId === "string" && sessionId) {
      pausedSessions.add(sessionId);
    }
  });
  let armed = false;
  try {
    await connection.send("Target.setAutoAttach", {
      autoAttach: true,
      waitForDebuggerOnStart: true,
      flatten: true,
      filter: WRITER_TARGET_FILTER,
    });
    armed = true;
    for (const sessionId of pageSessions) {
      frozenSessions.push(sessionId);
      await connection.send("ServiceWorker.enable", {}, sessionId);
      await connection.send("ServiceWorker.stopAllWorkers", {}, sessionId);
      await connection.send("Emulation.setScriptExecutionDisabled", { value: true }, sessionId);
      await connection.send("Network.emulateNetworkConditions", OFFLINE, sessionId);
      await connection.send("Page.stopLoading", {}, sessionId);
    }
    if (typeof waitForNetworkIdle !== "function") throw new Error("network idle guard unavailable");
    await waitForNetworkIdle(pageSessions);
    for (const sessionId of pageSessions) {
      await connection.send("Page.setWebLifecycleState", { state: "frozen" }, sessionId);
    }
    return new BrowserCookieWriterFence({
      connection,
      frozenSessions,
      pausedSessions,
      unsubscribe,
    });
  } catch (error) {
    unsubscribe();
    await release({ connection, frozenSessions, pausedSessions, armed }).catch(() => {});
    throw failure("browser_cookie_writer_fence_failed", error);
  }
}

class BrowserCookieWriterFence {
  constructor({ connection, frozenSessions, pausedSessions, unsubscribe }) {
    this.connection = connection;
    this.frozenSessions = frozenSessions;
    this.pausedSessions = pausedSessions;
    this.unsubscribe = unsubscribe;
    this.released = false;
  }

  async release() {
    if (this.released) return;
    this.released = true;
    this.unsubscribe();
    try {
      await release({
        connection: this.connection,
        frozenSessions: this.frozenSessions,
        pausedSessions: this.pausedSessions,
        armed: true,
      });
    } catch (error) {
      throw failure("browser_cookie_writer_fence_release_failed", error);
    }
  }
}

async function release({ connection, frozenSessions, pausedSessions, armed }) {
  const failures = [];
  if (armed) {
    await connection.send("Target.setAutoAttach", {
      autoAttach: false,
      waitForDebuggerOnStart: false,
      flatten: true,
    }).catch((error) => failures.push(error));
  }
  for (const sessionId of [...pausedSessions].reverse()) {
    await connection.send("Runtime.runIfWaitingForDebugger", {}, sessionId)
      .catch((error) => failures.push(error));
    await connection.send("Target.detachFromTarget", { sessionId })
      .catch((error) => failures.push(error));
  }
  for (const sessionId of [...frozenSessions].reverse()) {
    await connection.send("Page.setWebLifecycleState", { state: "active" }, sessionId)
      .catch((error) => failures.push(error));
    await connection.send("Network.emulateNetworkConditions", ONLINE, sessionId)
      .catch((error) => failures.push(error));
    await connection.send("Emulation.setScriptExecutionDisabled", { value: false }, sessionId)
      .catch((error) => failures.push(error));
    await connection.send("ServiceWorker.disable", {}, sessionId)
      .catch((error) => failures.push(error));
  }
  if (failures.length) throw failures[0];
}

function failure(code, cause) {
  return Object.assign(new Error(code), {
    code,
    recoveryRequired: true,
    ...(cause === undefined ? {} : { cause }),
  });
}
