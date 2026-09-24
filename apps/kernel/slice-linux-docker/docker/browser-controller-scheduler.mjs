const CONCURRENT_METHODS = new Set([
  "health",
  "browser.snapshot",
  "browser.wait",
  "browser.events.poll",
]);

const TAB_MUTATION_METHODS = new Set([
  "browser.action",
  "browser.navigate",
  "browser.history",
  "browser.dialog",
  "browser.upload",
]);

function requestScope(request) {
  if (CONCURRENT_METHODS.has(request?.method)) {
    return { kind: "concurrent" };
  }
  if (TAB_MUTATION_METHODS.has(request?.method)) {
    const targetId = request.params?.target_id;
    if (typeof targetId === "string" && targetId.length > 0) {
      return { kind: "tab", targetId };
    }
  }
  // Reconciliation, tab lifecycle, browser-wide configuration, cookie state,
  // downloads, shutdown, and unknown methods stay behind a browser-wide fence.
  return { kind: "barrier" };
}

export class BrowserControllerRequestScheduler {
  #queue = [];
  #activeCount = 0;
  #activeTabs = new Set();
  #barrierActive = false;
  #idleWaiters = new Set();

  schedule(request, operation) {
    if (typeof operation !== "function") {
      throw new TypeError("scheduled browser operation must be a function");
    }
    const scope = requestScope(request);
    return new Promise((resolve, reject) => {
      this.#queue.push({ scope, operation, resolve, reject });
      this.#pump();
    });
  }

  drain() {
    if (this.#isIdle()) return Promise.resolve();
    return new Promise((resolve) => this.#idleWaiters.add(resolve));
  }

  #pump() {
    if (this.#barrierActive) return;

    const barrierIndex = this.#queue.findIndex((entry) => entry.scope.kind === "barrier");
    const preceding = this.#queue.slice(0, barrierIndex < 0 ? this.#queue.length : barrierIndex);
    const blockedTabs = new Set();

    for (const entry of preceding) {
      const index = this.#queue.indexOf(entry);
      if (index < 0) continue;
      if (entry.scope.kind === "tab") {
        const { targetId } = entry.scope;
        if (blockedTabs.has(targetId) || this.#activeTabs.has(targetId)) {
          blockedTabs.add(targetId);
          continue;
        }
        blockedTabs.add(targetId);
      }
      this.#queue.splice(index, 1);
      this.#start(entry);
    }

    if (this.#queue[0]?.scope.kind === "barrier" && this.#activeCount === 0) {
      this.#start(this.#queue.shift());
    }
    this.#resolveIdle();
  }

  #start(entry) {
    this.#activeCount += 1;
    if (entry.scope.kind === "tab") this.#activeTabs.add(entry.scope.targetId);
    if (entry.scope.kind === "barrier") this.#barrierActive = true;

    Promise.resolve()
      .then(entry.operation)
      .then(entry.resolve, entry.reject)
      .finally(() => {
        this.#activeCount -= 1;
        if (entry.scope.kind === "tab") this.#activeTabs.delete(entry.scope.targetId);
        if (entry.scope.kind === "barrier") this.#barrierActive = false;
        this.#pump();
      });
  }

  #isIdle() {
    return this.#queue.length === 0 && this.#activeCount === 0;
  }

  #resolveIdle() {
    if (!this.#isIdle()) return;
    for (const resolve of this.#idleWaiters) resolve();
    this.#idleWaiters.clear();
  }
}
