// Keeps the person's unsaved scene in the App, not in this window: the view is replaced when the App
// updates or the connection returns, and its page keeps nothing across that. Each window keeps its
// draft under an id of its own, so another window's save or discard never reaches it; the new window
// asks for every kept draft and merges them into its canvas by element version.
const DELAY_MS = 100;

// `kept(n)` runs when the window's unsaved scene is safe in the App, or the window has none: the draft
// write numbered n was acknowledged, or there was nothing to write.
export function drafts(call, id, read, kept) {
  // One write at a time and in order, so a late write never outlives the discard that follows it.
  let queue = Promise.resolve();
  let timer = null;
  let reported = null;
  let written = false;
  let sends = 0;
  const run = (tool, input) => {
    const done = queue.then(() => call(tool, input));
    queue = done.catch(() => {});
    return done;
  };
  // The draft is best effort: its failure is told once in the console and never stops the save.
  const tell = error => {
    const message = `Could not keep the unsaved diagram: ${error?.message ?? error}`;
    if (message === reported) return;
    reported = message;
    console.error(message);
  };
  // `read` is asked when the write is due, so the latest edit is the one kept; it returns the scene
  // that differs from the saved one, or nothing when the window has no unsaved edit. The result is
  // true when the scene is in the App or there is none, false when the write was refused.
  const send = () => {
    clearTimeout(timer);
    timer = null;
    const scene = read();
    const number = ++sends;
    written = Boolean(scene);
    if (!scene) {
      kept(number);
      return run('discard_draft', { id }).catch(tell).then(() => true);
    }
    return run('save_draft', { id, scene }).then(() => { kept(number); return true; }, error => { tell(error); return false; });
  };
  return {
    // At most one write per delay however fast the person draws.
    // How many writes were started: a write numbered above it reads what the window has from now on.
    started: () => sends,
    keep() { written = true; timer ??= setTimeout(send, DELAY_MS); },
    // A window going away or recovering writes what it has now.
    flush() { return timer ? send() : Promise.resolve(true); },
    // The window saved what it had: its draft, and the write that was due, are no longer needed.
    drop() {
      clearTimeout(timer);
      timer = null;
      if (!written) return Promise.resolve();
      written = false;
      return run('discard_draft', { id }).catch(tell);
    },
    async recall() {
      await queue;
      return (await call('read_drafts', {})).drafts;
    },
    // Drafts of windows that were replaced, once their edits are saved or kept by this window.
    forget: ids => Promise.all(ids.map(other => run('discard_draft', { id: other }).catch(tell))),
  };
}
