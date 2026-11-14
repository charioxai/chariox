// The change feed a backend-served view reads (`view_changes`), written in the same state transaction as
// the App's own writes so a reader never sees a change that did not commit. Same keys and limits as the
// Rust SDK's `transact_with_change`.
export const CHANGE_LIMIT = 128;
// One change as stored (`seq` and `delta`), so the feed stays within 1 MiB.
export const CHANGE_BYTES = 8 * 1024;
const LATEST_KEY = 'chariox.view.latest';
const changeKey = seq => `chariox.view.change.${seq}`;
const failure = (code, message) => ({ code, message, retryable: false });

function head(record) {
  const seq = Number(record.value);
  if (!Number.isSafeInteger(seq) || seq < 0) throw failure('INVALID_STATE', 'the change feed head is not a number');
  return seq;
}

export function createFeed(state) {
  return {
    /** Commit `tx` together with one change; `delta` is what a reader applies to its copy. */
    async transact(tx, delta) {
      const latest = await state.get(LATEST_KEY);
      const seq = (latest ? head(latest) : 0) + 1;
      const event = JSON.stringify({ seq, delta });
      const bytes = new TextEncoder().encode(event).length;
      if (bytes > CHANGE_BYTES) throw failure('CHANGE_TOO_LARGE', `a change is at most ${CHANGE_BYTES} bytes, this one is ${bytes}`);
      const writes = [...tx.writes, { tag: 'put', val: [LATEST_KEY, String(seq)] }, { tag: 'put', val: [changeKey(seq), event] }];
      if (seq > CHANGE_LIMIT) writes.push({ tag: 'delete', val: changeKey(seq - CHANGE_LIMIT) });
      return state.transact({ ...tx, checks: [...tx.checks, { key: LATEST_KEY, version: latest?.version ?? null }], writes });
    },

    /** The `view_changes` answer: the head, whether `since` fell out of the kept window, and the changes after it. */
    async changes({ since = 0 } = {}) {
      const latestRecord = await state.get(LATEST_KEY);
      const latest = latestRecord ? head(latestRecord) : 0;
      const oldest = Math.max(latest - (CHANGE_LIMIT - 1), 1);
      const first = Math.max(since + 1, oldest);
      const changes = [];
      for (let seq = first; seq <= latest; seq++) {
        const record = await state.get(changeKey(seq));
        if (!record) throw failure('INVALID_STATE', `change ${seq} is missing`);
        changes.push(JSON.parse(record.value));
      }
      return { latest, dropped: since + 1 < oldest, changes };
    },
  };
}
