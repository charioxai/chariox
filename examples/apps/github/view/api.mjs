// Calls to the App's backend. The window never talks to GitHub: every call is a tool of the App, run by the
// kernel as one of the owner's accounts.
const inflight = new Map();
const kept = new Map();

const key = (name, input) => `${name}\n${JSON.stringify(input)}`;

/** One call; the same call made while it runs shares it. */
export function call(name, input = {}) {
  const k = key(name, input);
  if (!inflight.has(k)) inflight.set(k, globalThis.chariox.call(name, input).finally(() => inflight.delete(k)));
  return inflight.get(k);
}

/** The answer kept from the last call, if any, so a page revisited shows at once and then refreshes. */
export const peek = (name, input = {}) => kept.get(key(name, input));

export async function load(name, input = {}) {
  const value = await call(name, input);
  kept.set(key(name, input), value);
  return value;
}

export const forget = () => kept.clear();

/** A failure as the window words it. */
export function describe(error) {
  const code = error?.code ?? 'CALL_FAILED';
  if (code === 'ACCOUNT_NOT_BOUND') return { code, title: 'Connect a GitHub account', text: 'This App has no GitHub account yet. The owner of this kernel binds one in the App\'s permissions.' };
  if (code === 'ACCOUNT_UNAUTHORIZED') return { code, title: 'GitHub did not accept the account', text: 'Its token may have expired or been revoked.' };
  if (code === 'ACCOUNT_UNAVAILABLE') return { code, title: 'The account cannot be used right now', text: 'Unlock the kernel\'s Vault.' };
  if (code === 'RATE_LIMITED') return { code, title: 'API rate limit exceeded', text: error.message };
  if (code === 'NOT_FOUND') return { code, title: 'Not found', text: 'This does not exist, or the account cannot see it.' };
  return { code, title: 'Something went wrong', text: String(error?.message ?? error) };
}
