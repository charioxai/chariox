// Keeps the person's unsaved edit in the App, not in this window: the view is replaced when the App
// updates or the connection returns, and its sandbox keeps nothing across that. The App holds one
// draft, the open document's, with the revision the edit started from.
const DELAY_MS = 400

export function drafts(call) {
  // One write at a time and in order, so a late save never outlives the discard that follows it.
  let queue = Promise.resolve()
  let pending = null
  const run = (tool, input) => {
    const done = queue.then(() => call(tool, input))
    queue = done.catch(() => {})
    return done
  }
  function send(read) {
    const draft = read()
    if (draft) run("save_draft", draft).catch(() => {})
  }
  return {
    // `read` is asked when the write is due, so the latest keystrokes are the ones kept.
    keep(read) {
      clearTimeout(pending?.timer)
      pending = { read, timer: setTimeout(() => { pending = null; send(read) }, DELAY_MS) }
    },
    // A window going away writes what it has now; the write may not finish, which the delay bounds.
    flush() {
      if (!pending) return
      clearTimeout(pending.timer)
      const { read } = pending
      pending = null
      send(read)
    },
    drop() {
      clearTimeout(pending?.timer)
      pending = null
      return run("discard_draft", {}).catch(() => {})
    },
    async recall() {
      await queue
      return (await call("read_draft", {})).draft
    },
  }
}
