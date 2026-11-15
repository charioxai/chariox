// The accounts the App uses with what GitHub says about each: who it is and how much of its rate limit is left.
import { RESERVE } from './github.mjs';

const user = data => ({ login: data.login, name: data.name ?? null, avatarUrl: data.avatar_url });
/** The reason an account does not work, in words the window can state. */
const failure = error => {
  const cause = error?.payload ?? error;
  const code = cause?.code ?? 'ERROR';
  return { status: code === 'ACCOUNT_NOT_BOUND' ? 'unbound' : code === 'ACCOUNT_UNAUTHORIZED' ? 'unauthorized'
    : code === 'RATE_LIMITED' ? 'rate_limited' : 'unavailable', code, message: String(cause?.message ?? error), resetAt: cause?.resetAt ?? null };
};

export function createStatus({ accounts, github }) {
  const resources = async label => {
    const [core, search] = await Promise.all(['core', 'search'].map(resource => github.rate(label, resource)));
    return { core: core && { limit: core.limit, remaining: core.remaining, resetAt: core.reset * 1000 }, search: search && { limit: search.limit, remaining: search.remaining, resetAt: search.reset * 1000 } };
  };

  async function describe(label, isDefault) {
    try {
      const { data } = await github.get(label, '/user', {});
      return { label, isDefault, ...user(data), status: 'ok', rate: await resources(label) };
    } catch (error) {
      return { label, isDefault, ...failure(error), rate: await resources(label) };
    }
  }

  return {
    async list_accounts() {
      const settings = await accounts.settings();
      const known = settings.labels.length ? settings.labels : [];
      return {
        accounts: await Promise.all(known.map(label => describe(label, label === (settings.default ?? known[0])))),
        default: settings.default ?? known[0] ?? null, repos: settings.repos,
      };
    },

    /** `/rate_limit` is free; it shows what GitHub counts now rather than what the App last saw. */
    async get_status({ account } = {}) {
      const settings = await accounts.settings();
      const labels = account ? [account] : settings.labels;
      const read = async label => {
        try {
          const { data } = await github.get(label, '/rate_limit', {}, { fresh: true });
          const pick = r => ({ limit: r.limit, remaining: r.remaining, resetAt: r.reset * 1000, paused: r.remaining < RESERVE.core });
          return { label, status: 'ok', core: pick(data.resources.core), search: { ...pick(data.resources.search), paused: data.resources.search.remaining < RESERVE.search } };
        } catch (error) { return { label, ...failure(error) }; }
      };
      return { accounts: await Promise.all(labels.map(read)) };
    },
  };
}
