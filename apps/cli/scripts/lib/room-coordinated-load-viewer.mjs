import { setImmediate as yieldTurn } from "node:timers/promises"

// Drain an active video stream continuously; retain only the latest frame.
export function startLoadViewerReader(stream, readFrame) {
  let latest = null
  let failure = null
  let stopped = false
  let paused = false
  let resume = null
  let changed = null
  const notify = () => { changed?.(); changed = null }
  const finished = (async () => {
    try {
      while (!stopped) {
        if (paused) await new Promise((resolve) => { resume = resolve })
        if (stopped) break
        latest = await readFrame(stream)
        notify()
        await yieldTurn()
      }
    } catch (error) {
      if (!stopped) failure = error
      notify()
    }
  })()
  return {
    async takeFrame() {
      while (!latest && !failure && !stopped) await new Promise((resolve) => { changed = resolve })
      if (failure) throw failure
      if (stopped) throw new Error("owned viewer reader is stopped")
      const frame = latest
      latest = null
      return frame
    },
    pause() { paused = true },
    resume() { paused = false; resume?.(); resume = null },
    async stop() {
      stopped = true
      paused = false
      resume?.()
      notify()
      await stream.close()
      await finished
    },
  }
}
