// Keeps the person's unsaved scene in the App, not in this window: the view is replaced when the App
// updates or the connection returns, and its page keeps nothing across that. Each window keeps its
// draft under an id of its own and only that window writes or removes it; a draft is otherwise removed
// only when the backend finds it older than an hour. The new window lists the kept drafts, reads
// them one at a time (all of them together can exceed the catalog's call limit) and merges them into
// its canvas by element version. It removes another window's draft only once its own draft or the saved
// scene holds what it merged, and only that revision: a window that has written since keeps its draft.
const DELAY_MS = 100;

// `notice` tells the person when a write is refused or fails and when writes work again.
export function drafts(call, id, read, notice) {
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
  // The draft is best effort: its failure is told once, to the person and in the console, and never stops the save.
  const tell = error => {
    const message = `Could not keep the unsaved diagram: ${error?.message ?? error}`;
    if (message === reported) return false;
    reported = message;
    console.error(message);
    notice.failed(message);
    return false;
  };
  const worked = () => {
    if (reported) { reported = null; notice.recovered(); }
    return true;
  };
  // `read` is asked when the write is due, so the latest edit is the one kept; it returns the scene
  // that differs from the saved one, or nothing when the window has no unsaved edit.
  const send = () => {
    clearTimeout(timer);
    timer = null;
    const scene = read();
    kept = Boolean(scene);
    return (scene ? run('save_draft', { id, scene }) : run('discard_draft', { id })).then(worked, tell);
  };
  return {
    // At most one write per delay however fast the person draws.
    keep() { kept = true; timer ??= setTimeout(send, DELAY_MS); },
    // A window going away or recovering writes what it has now; true when that write was kept.
    flush() { return timer ? send() : Promise.resolve(true); },
    // The window saved what it had: its draft, and the write that was due, are no longer needed.
    drop() {
      clearTimeout(timer);
      timer = null;
      if (!kept) return Promise.resolve();
      kept = false;
      return run('discard_draft', { id }).then(worked, tell);
    },
    // The kept drafts of the other windows as {id, revision, scene}, oldest first, one read each.
    async recall() {
      await queue;
      const recalled = [];
      for (const { id: other } of (await call('list_drafts', {})).drafts) {
        const { revision, scene } = await call('read_draft', { id: other });
        if (scene) recalled.push({ id: other, revision, scene });
      }
      return recalled;
    },
    // What this window merged is held by its own draft or the saved scene: forget exactly those revisions.
    async retire(recalled) {
      for (const { id: other, revision } of recalled) await run('retire_draft', { id: other, revision }).catch(tell);
    },
  };
}
