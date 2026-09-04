export function createDrillInterruption(signals = process) {
  const abort = new AbortController()
  const clients = new WeakSet()
  let cleaning = false
  let started = false
  const check = () => { if (!cleaning) abort.signal.throwIfAborted() }
  const interrupt = (signal) => {
    if (!cleaning && !abort.signal.aborted) abort.abort(new Error(`drill interrupted by ${signal}`))
  }
  const handlers = { SIGINT: () => interrupt("SIGINT"), SIGTERM: () => interrupt("SIGTERM") }
  return {
    check,
    guardClient(client) {
      if (!clients.has(client)) {
        const send = client.send.bind(client)
        client.send = async (...args) => { check(); return send(...args) }
        clients.add(client)
      }
      return client
    },
    sleep(ms) {
      check()
      if (cleaning) return new Promise((resolve) => setTimeout(resolve, ms))
      return new Promise((resolve, reject) => {
        const interrupted = () => { clearTimeout(timer); reject(abort.signal.reason) }
        const timer = setTimeout(() => {
          abort.signal.removeEventListener("abort", interrupted)
          resolve()
        }, ms)
        abort.signal.addEventListener("abort", interrupted, { once: true })
      })
    },
    async run(work, cleanup, recordFailure) {
      if (started) throw new Error("drill lifecycle already started")
      started = true
      for (const [signal, handler] of Object.entries(handlers)) signals.on(signal, handler)
      try {
        try { await work(); check() } catch (error) { recordFailure(error) }
        finally {
          // Do not race cleanup against work still provisioning a resource.
          // Keep handlers installed so repeated signals cannot interrupt cleanup.
          cleaning = true
          await cleanup()
        }
      } finally {
        for (const [signal, handler] of Object.entries(handlers)) signals.removeListener(signal, handler)
      }
    },
  }
}
