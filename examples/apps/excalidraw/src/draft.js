// Keeps the person's unsaved scene in the App, not in this window: the view is replaced when the App
// updates or the connection returns, and its page keeps nothing across that. Each window keeps one
// draft, its whole scene, under an id of its own, and each write replaces it. Only that window writes
// or discards it. Another window removes it only by retiring the revision it read once the saved scene
// holds what it merged, and the backend drops a draft older than an hour. A recovering window lists
// the kept drafts and reads them one at a time, since all of them together can exceed the catalog's
// call limit.
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
  // The draft is best effort: its failure is told once, to the person and in the console, and never
  // stops a save or a recovery.
  const tell = error => {
    const message = `Could not keep the unsaved diagram: ${error?.message ?? error}`;
    if (message === reported) return;
    reported = message;
    console.error(message);
    notice.failed(message);
  };
  const worked = () => {
    if (reported) { reported = null; notice.recovered(); }
  };
  // `read` is asked when the write is due, so the latest edit is the one kept; it returns the window's
  // whole scene, or nothing when the window has no unsaved edit.
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
    // A window going away writes what it has now.
    flush() { return timer ? send() : Promise.resolve(); },
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
    // The saved scene holds what this window merged: forget exactly the revisions it read.
    async retire(recalled) {
      for (const { id: other, revision } of recalled) await run('retire_draft', { id: other, revision }).catch(tell);
    },
  };
}
