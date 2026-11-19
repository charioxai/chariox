// Which Room (and agent) made a pull request, so a pull request can be traced back to the work that produced it.
// The App keeps only ids the kernel gave the call; the newest 500 links stay, the oldest go.
const KEY = 'links';
const MAX_LINKS = 500;
const keyOf = (repo, number) => `${repo.toLowerCase()}#${number}`;

export function createLinks({ state, view, now }) {
  const read = async () => {
    const record = await state.get(KEY);
    return { value: record ? JSON.parse(record.value) : {}, version: record?.version ?? null };
  };

  return {
    async get(repo, number) {
      const link = (await read()).value[keyOf(repo, number)];
      return link ? { roomId: link.roomId, agentId: link.agentId, branch: link.branch, at: link.at } : null;
    },

    /** The calling Room made this pull request; a call outside a Room links nothing. */
    async put(ctx, { repo, number, branch }) {
      if (!ctx.roomId) return null;
      const link = { roomId: ctx.roomId, agentId: ctx.agentId ?? null, branch: branch ?? null, at: now() };
      let last;
      for (let attempt = 0; attempt < 4; attempt++) {
        const { value, version } = await read();
        const entries = Object.entries({ ...value, [keyOf(repo, number)]: link }).sort((a, b) => a[1].at - b[1].at).slice(-MAX_LINKS);
        try {
          await view.transactWithChange({
            schemaVersion: 0, checks: [{ key: KEY, version }],
            writes: [{ tag: 'put', val: [KEY, JSON.stringify(Object.fromEntries(entries))] }], occurrences: [], wakes: [],
          }, { kind: 'event', repo, number });
          return link;
        } catch (error) { last = error; }
      }
      throw last;
    },
  };
}
