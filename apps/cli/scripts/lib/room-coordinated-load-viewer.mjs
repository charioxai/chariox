import { setImmediate as yieldTurn } from "node:timers/promises"

// Drain an active video stream continuously; retain only the latest frame.
export function startLoadViewerReader(stream, readFrame) {
  let latest = null
  let failure = null
  let stopped = false
  let paused = false
  let resume = null
  let pauseReady = null
  let changed = null
  const notify = () => { changed?.(); changed = null }
  const finished = (async () => {
    try {
      while (!stopped) {
        if (paused) {
          pauseReady?.(); pauseReady = null
          await new Promise((resolve) => { resume = resolve })
        }
        if (stopped) break
        latest = await readFrame(stream)
        notify()
        await yieldTurn()
      }
    } catch (error) {
      if (!stopped) failure = error
      pauseReady?.(); pauseReady = null
      notify()
    }
  })()
  return {
    async takeFrame({ fresh = false } = {}) {
      if (fresh) latest = null
      while (latest === null && !failure && !stopped) await new Promise((resolve) => { changed = resolve })
      if (failure) throw failure
      if (stopped) throw new Error("owned viewer reader is stopped")
      const frame = latest
      latest = null
      return frame
    },
    pause() {
      if (failure || stopped) return Promise.resolve()
      paused = true
      return new Promise((resolve) => { pauseReady = resolve })
    },
    resume() { paused = false; resume?.(); resume = null },
    async stop() {
      stopped = true
      paused = false
      resume?.()
      pauseReady?.()
      notify()
      await stream.close()
      await finished
    },
  }
}
