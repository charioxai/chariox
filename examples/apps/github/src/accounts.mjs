// Which of the owner's GitHub accounts the App uses. The owner binds an account (a label for a Vault entry)
// in the kernel; the App keeps only labels, never anything secret: which accounts it knows, which is the
// default and which account a repository uses.
const KEY = 'accounts';
const fail = (code, message) => { throw { code, message, retryable: false }; };
/** The label the first binding gets when the owner accepts the App's suggestion. */
export const FIRST_LABEL = 'github';
const initial = () => ({ default: null, labels: [FIRST_LABEL], repos: {} });

export function createAccounts({ state, view }) {
  async function read() {
    const record = await state.get(KEY);
    return { value: record ? JSON.parse(record.value) : initial(), version: record?.version ?? null };
  }

  // The change feed and the record commit together; a concurrent writer makes this retry on what it left.
  async function update(mutate) {
    let last;
    for (let attempt = 0; attempt < 4; attempt++) {
      const { value, version } = await read();
      const next = mutate(JSON.parse(JSON.stringify(value)));
      try {
        await view.transactWithChange({
          schemaVersion: 0, checks: [{ key: KEY, version }],
          writes: [{ tag: 'put', val: [KEY, JSON.stringify(next)] }], occurrences: [], wakes: [],
        }, { kind: 'accounts' });
        return next;
      } catch (error) { last = error; }
    }
    throw last;
  }

  return {
    settings: async () => (await read()).value,

    /** The account a call uses: the one named, the repository's, the default, else the first known. */
    async resolve({ account, repo } = {}) {
      const { value } = await read();
      const label = account ?? (repo ? value.repos[repo.toLowerCase()] : undefined) ?? value.default ?? value.labels[0];
      if (!label) fail('ACCOUNT_NOT_BOUND', 'No GitHub account is set up for this App. The owner binds one in the kernel.');
      return label;
    },

    async add(label) {
      return update(value => {
        value.default ??= value.labels[0] ?? label;
        if (!value.labels.includes(label)) value.labels.push(label);
        return value;
      });
    },

    async remove(label) {
      return update(value => {
        value.labels = value.labels.filter(known => known !== label);
        if (value.default === label) value.default = value.labels[0] ?? null;
        for (const [repo, account] of Object.entries(value.repos)) if (account === label) delete value.repos[repo];
        return value;
      });
    },

    async setDefault(label) {
      return update(value => {
        if (!value.labels.includes(label)) fail('NOT_FOUND', `The App does not use an account named "${label}".`);
        value.default = label;
        return value;
      });
    },

    async setRepo(repo, label) {
      return update(value => {
        if (label !== undefined && !value.labels.includes(label)) fail('NOT_FOUND', `The App does not use an account named "${label}".`);
        if (label === undefined) delete value.repos[repo.toLowerCase()]; else value.repos[repo.toLowerCase()] = label;
        return value;
      });
    },
  };
}
