// Keeps the person's unsaved scene in the App, not in this window: the view is replaced when the App
// updates or the connection returns, and its page keeps nothing across that. Each window keeps its
// draft under an id of its own and only that window writes or removes it; a draft is otherwise removed
// only when the backend finds it older than an hour. The new window asks for every kept draft and
// merges them into its canvas by element version: it copies, so it never removes another window's.
const DELAY_MS = 100;

export function drafts(call, id, read) {
  // One write at a time and in order, so a late write never outlives the discard that follows it.
  let queue = Promise.resolve();
  let timer = null;
  let reported = null;
  let kept = false;
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
  // that differs from the saved one, or nothing when the window has no unsaved edit.
  const send = () => {
    clearTimeout(timer);
    timer = null;
    const scene = read();
    kept = Boolean(scene);
    return (scene ? run('save_draft', { id, scene }) : run('discard_draft', { id })).catch(tell);
  };
  return {
    // At most one write per delay however fast the person draws.
    keep() { kept = true; timer ??= setTimeout(send, DELAY_MS); },
    // A window going away or recovering writes what it has now.
    flush() { return timer ? send() : Promise.resolve(); },
    // The window saved what it had: its draft, and the write that was due, are no longer needed.
    drop() {
      clearTimeout(timer);
      timer = null;
      if (!kept) return Promise.resolve();
      kept = false;
      return run('discard_draft', { id }).catch(tell);
    },
    async recall() {
      await queue;
      return (await call('read_drafts', {})).drafts;
    },
  };
}
