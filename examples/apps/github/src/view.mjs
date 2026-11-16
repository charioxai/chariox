// The backend serves the window: the signed actions, the state and a revisioned change feed that is
// committed in the same transaction as the App's own write, so a reader never sees a change that did not
// commit. The Rust SDK's `View` has the same keys and limits.
const LATEST = 'chariox.view.latest';
const changeKey = seq => `chariox.view.change.${seq}`;
/** Changes the feed keeps; an older reader is told to read the state again. */
export const CHANGE_LIMIT = 128;
/** One change as stored, so a whole feed stays within 1 MiB. */
export const CHANGE_BYTES = 8 * 1024;

const fail = (code, message) => { throw { code, message, retryable: false }; };

export function createView({ state, actions, snapshot, account }) {
  async function transactWithChange(tx, delta) {
    const latest = await state.get(LATEST);
    const seq = (latest ? Number(latest.value) : 0) + 1;
    if (!Number.isSafeInteger(seq)) fail('INVALID_STATE', 'the change feed head is not a number');
    const event = JSON.stringify({ seq, delta });
    if (event.length > CHANGE_BYTES) fail('CHANGE_TOO_LARGE', `a change is at most ${CHANGE_BYTES} bytes, this one is ${event.length}`);
    const writes = [...tx.writes,
      { tag: 'put', val: [LATEST, String(seq)] }, { tag: 'put', val: [changeKey(seq), event] }];
    if (seq > CHANGE_LIMIT) writes.push({ tag: 'delete', val: changeKey(seq - CHANGE_LIMIT) });
    return state.transact({ ...tx, checks: [...tx.checks, { key: LATEST, version: latest?.version ?? null }], writes });
  }

  async function changes({ since = 0 } = {}) {
    const head = await state.get(LATEST);
    const latest = head ? Number(head.value) : 0;
    const oldest = Math.max(latest - (CHANGE_LIMIT - 1), 1);
    const first = Math.max(since + 1, oldest);
    const list = [];
    for (let seq = first; seq <= latest; seq++) {
      const record = await state.get(changeKey(seq));
      if (!record) fail('INVALID_STATE', `change ${seq} is missing`);
      list.push(JSON.parse(record.value));
    }
    return { latest, dropped: since + 1 < oldest, changes: list };
  }

  return {
    transactWithChange,
    tools: {
      view_actions: async () => actions,
      view_state: async () => snapshot(),
      view_account: async input => ({ account: await account(input ?? {}) }),
      view_changes: input => changes(input),
    },
  };
}
