// MP-08 / MP-09 / MP-10 / MP-11: the probe and the in-place credential replacement, against stand-in network and kernel.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { probeClaudeToken, replaceClaudeCredential, readToken } from './claude-account.mjs';

const reply = (status, headers) => async () => new Response('{}', { status, headers });
const org = 'aaaaaaaa-bbbb-cccc-dddd-0123456789ab';

test('an allowed 200 is usable and reports only the organization suffix and status', async () => {
  const seen = [];
  const result = await probeClaudeToken('sk-secret-token', { fetchImpl: async (url, init) => { seen.push(init.headers.authorization); return reply(200, { 'anthropic-organization-id': org, 'anthropic-ratelimit-unified-status': 'allowed' })(); } });
  assert.deepEqual(result, { usable: true, http: 200, org: '0123456789ab', status: 'allowed' });
  assert.equal(seen[0], 'Bearer sk-secret-token');
  assert.ok(!JSON.stringify(result).includes('secret'));
});

test('a rejected weekly limit, a non-200 and a network failure are all unusable with a reason', async () => {
  const rejected = await probeClaudeToken('t', { fetchImpl: reply(200, { 'anthropic-organization-id': org, 'anthropic-ratelimit-unified-status': 'rejected' }) });
  assert.equal(rejected.usable, false); assert.match(rejected.reason, /limit status rejected/);
  const unauthorized = await probeClaudeToken('t', { fetchImpl: reply(401, {}) });
  assert.equal(unauthorized.usable, false); assert.equal(unauthorized.reason, 'HTTP 401');
  const down = await probeClaudeToken('sk-secret-token', { fetchImpl: async () => { throw new TypeError('connect sk-secret-token'); } });
  assert.equal(down.usable, false); assert.ok(!JSON.stringify(down).includes('secret'));
});

test('replacing the credential overwrites the stored one and reports the auth state without the value', async () => {
  const calls = [];
  const rpc = async request => { calls.push(request); return Object.keys(request)[0] === 'GetProviderAuthStatus' ? { ProviderAuthStatus: { auth_state: 'authenticated' } } : { ok: true }; };
  const auth = await replaceClaudeCredential({ rpc, profileId: 'profile-1', token: 'sk-new' });
  assert.deepEqual(calls[0], { SetProviderAccountCredential: { provider: 'claude', account_profile: 'profile-1', value: 'sk-new', overwrite: true } });
  assert.deepEqual(auth, ['"auth_state":"authenticated"']);
  assert.ok(!JSON.stringify(auth).includes('sk-new'));
});

test('the probe command refuses an empty token file and a missing argument', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'claude-account-')); t.after(() => rm(dir, { recursive: true, force: true }));
  const empty = join(dir, 'empty'); await writeFile(empty, '\n');
  assert.throws(() => readToken(empty), /is empty/);
  const script = new URL('./claude-account.mjs', import.meta.url).pathname;
  assert.equal(spawnSync(process.execPath, [script, 'probe'], { encoding: 'utf8' }).status, 2);
});
