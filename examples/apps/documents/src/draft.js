// Keeps the person's unsaved edit in the App, not in this window: the view is replaced when the App
// updates or the connection returns, and its sandbox keeps nothing across that. The App holds one
// draft per (document, editor), with the revision the edit started from. A window has no identity that
// outlives its replacement, so each window instance is a new editor with a random name: its saves and
// discards reach only its own drafts, and its first `recall` is the one place a draft changes hands.
const DELAY_MS = 400

export const newEditor = () => Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, "0")).join("")

export function drafts(call, editor) {
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
    if (draft) run("save_draft", { editor, ...draft }).catch(() => {})
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
    // This editor's draft of the document `id`; a pending write is this window's, so it goes with it.
    drop(id) {
      clearTimeout(pending?.timer)
      pending = null
      return run("discard_draft", { editor, id }).catch(() => {})
    },
    // The draft this window starts with: its own, else the latest of a replaced window, moved to this one.
    async recall() {
      await queue
      return (await call("claim_draft", { editor })).draft
    },
  }
}
