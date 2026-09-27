// Drill-local observation only: this never owns or mutates product resources.
export function createManagedParityObserverHooks({ timeoutMs = 45_000 } = {}) {
  let observer = null
  let failure = null
  let admitted = false
  return {
    bind(value) {
      if (observer || admitted) throw new Error("managed parity observer must be bound once before product admission")
      if (typeof value?.observeCreated !== "function" || typeof value?.beforeRetire !== "function") {
        throw new Error("managed parity observer lifecycle hooks are required")
      }
      observer = value
    },
    admit({ cleanup = false } = {}) {
      admitted = true
      if (failure && !cleanup) throw failure
    },
    async observe(method, receipt, { cleanup = false, signal } = {}) {
      if (!observer) return
      if (failure && !cleanup) throw failure
      const controller = new AbortController()
      const abort = () => controller.abort(signal.reason)
      signal?.addEventListener("abort", abort, { once: true })
      if (signal?.aborted) abort()
      let timer
      try {
        await Promise.race([
          Promise.resolve().then(() => observer[method](structuredClone(receipt), { signal: controller.signal })),
          new Promise((_, reject) => {
            timer = setTimeout(() => {
              controller.abort()
              reject(new Error("managed parity independent observer deadline exceeded"))
            }, timeoutMs)
          }),
        ])
      } catch (error) {
        failure ??= error instanceof Error ? error : new Error("managed parity independent observer failed")
        if (!cleanup) throw failure
      } finally {
        clearTimeout(timer)
        signal?.removeEventListener("abort", abort)
      }
    },
    assertHealthy() { if (failure) throw failure },
  }
}
