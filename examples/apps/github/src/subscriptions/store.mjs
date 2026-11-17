// Subscriptions and the last deliveries, each one record of App state changed with compare-and-set.
// A change commits together with its entry in the window's change feed, so an open window follows it.
export const LIMITS = { subscriptions: 100, deliveries: 200, bytes: 256 * 1024 };
const SUBSCRIPTIONS = 'subscriptions';
const DELIVERIES = 'deliveries';
const RELEASES = 'releases';
const fail = (code, message) => { throw { code, message, retryable: false }; };

export function createStore({ state, commit }) {
  async function read(key, empty) {
    const record = await state.get(key);
    return { value: record ? JSON.parse(record.value) : empty, version: record?.version ?? null };
  }

  /** Apply `mutate` to a copy and commit it; a concurrent writer makes this run again on what it left. */
  async function update(key, empty, mutate, delta) {
    let last;
    for (let attempt = 0; attempt < 5; attempt++) {
      const { value, version } = await read(key, empty);
      const draft = JSON.parse(JSON.stringify(value));
      const result = await mutate(draft);
      const text = JSON.stringify(draft);
      if (text.length > LIMITS.bytes) fail('LIMIT', `${key} may hold at most ${LIMITS.bytes / 1024} KiB`);
      try {
        await commit({ schemaVersion: 0, checks: [{ key, version }], writes: [{ tag: 'put', val: [key, text] }], occurrences: [], wakes: [] }, delta);
        return result;
      } catch (error) {
        if ((error?.payload ?? error)?.code !== 'CONFLICT') throw error;
        last = error;
      }
    }
    throw last;
  }

  return {
    subscriptions: async () => (await read(SUBSCRIPTIONS, [])).value,
    updateSubscriptions: mutate => update(SUBSCRIPTIONS, [], mutate, { kind: 'subscriptions' }),
    /** Automations of removed subscriptions that the kernel has not yet let go of. */
    releases: async () => (await read(RELEASES, [])).value,
    updateReleases: mutate => update(RELEASES, [], mutate, { kind: 'releases' }),
    deliveries: async () => (await read(DELIVERIES, [])).value,
    /** Newest last; the oldest records leave when the ring is full. */
    addDeliveries: records => records.length === 0 ? null : update(DELIVERIES, [], ring => {
      ring.push(...records);
      ring.splice(0, Math.max(0, ring.length - LIMITS.deliveries));
    }, { kind: 'deliveries' }),
  };
}
