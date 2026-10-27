// MP-08 / MP-09 / MP-10 / MP-11: the lanes' Claude account, checked and re-stored without ever printing a token.
// Usage: claude-account.mjs probe <token-file>   (exit 0 usable, 3 not usable, 2 usage)
import { readFileSync } from 'node:fs';
import { basename } from 'node:path';

const MESSAGES = 'https://api.anthropic.com/v1/messages';

// One haiku token, the same request claude-acct.sh probe makes. Only the organization id suffix and the limit status come back.
export async function probeClaudeToken(token, { fetchImpl = fetch } = {}) {
  let response;
  try {
    response = await fetchImpl(MESSAGES, {
      method: 'POST',
      signal: AbortSignal.timeout(30_000),
      headers: { authorization: `Bearer ${token}`, 'anthropic-version': '2023-06-01', 'anthropic-beta': 'oauth-2025-04-20', 'content-type': 'application/json' },
      body: JSON.stringify({ model: 'claude-haiku-5-5', max_tokens: 1, system: "You are Claude Code, Anthropic's official CLI for Claude.", messages: [{ role: 'user', content: 'hi' }] }),
    });
  } catch (error) {
    return { usable: false, http: null, org: null, status: null, reason: `request failed: ${String(error.name)}` };
  }
  await response.body?.cancel().catch(() => {});
  const org = response.headers.get('anthropic-organization-id')?.split('-').pop() ?? null;
  const status = response.headers.get('anthropic-ratelimit-unified-status');
  // allowed_warning means near the limit, not over it: the old account is deliberately paced there.
  const usable = response.status === 200 && (status === 'allowed' || status === 'allowed_warning');
  const reason = usable ? undefined : response.status !== 200 ? `HTTP ${response.status}` : `limit status ${status ?? 'missing'}`;
  return { usable, http: response.status, org, status, ...(reason && { reason }) };
}

export const readToken = path => {
  const token = readFileSync(path, 'utf8').trim();
  if (!token) throw new Error(`${basename(path)} is empty`);
  return token;
};

// The kernel keeps the credential in its Vault and hands it to a Claude process when that process is launched.
export async function replaceClaudeCredential({ rpc, profileId, token }) {
  await rpc({ SetProviderAccountCredential: { provider: 'claude', account_profile: profileId, value: token, overwrite: true } });
  const auth = JSON.stringify(await rpc({ GetProviderAuthStatus: { provider: 'claude', account_profile: profileId } }));
  return (auth.match(/"(?:auth_)?(?:state|status)":"[a-z_]+"/g) || []).slice(0, 2);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [command, file] = process.argv.slice(2);
  if (command !== 'probe' || !file) { console.error('usage: claude-account.mjs probe <token-file>'); process.exit(2); }
  const result = await probeClaudeToken(readToken(file));
  console.log(JSON.stringify({ file: basename(file), ...result }));
  process.exit(result.usable ? 0 : 3);
}
