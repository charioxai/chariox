// MP-08 / MP-10: the labelled Claude profile and the command arguments, against a stand-in kernel.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseCredentialArgs, storeClaudeProfile } from './claude-profile.mjs';

function kernel(profiles = []) {
  const calls = [];
  const rpc = async request => {
    const [name, body] = Object.entries(request)[0];
    calls.push(request);
    if (name === 'ListProviderAccountProfiles') return { ProviderAccountProfilesListed: { profiles: profiles.map(p => ({ provider: 'claude', ...p })) } };
    if (name === 'CreateProviderAccountProfile') { profiles.push({ provider: 'claude', label: body.label, profile_id: `${body.label}-new1`, origin: 'chariox_created' }); return { ok: true }; }
    if (name === 'GetProviderAuthStatus') return { ProviderAuthStatus: { auth_state: 'authenticated' } };
    return { ok: true };
  };
  return { calls, rpc, profiles };
}
async function sources(t) {
  const dir = await mkdtemp(join(tmpdir(), 'claude-profile-')); t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, 'defs')); await writeFile(join(dir, 'defs/scout.md'), 'scout definition');
  return { dir, agentSource: join(dir, 'defs'), agentNames: ['scout'], configDirectory: id => join(dir, 'config', id) };
}

test('a missing label is created, its credential stored with overwrite, and the worker agents synced into its config directory', async t => {
  const k = kernel([{ label: 'lanes-claude', profile_id: 'lanes-claude-aaaa', origin: 'chariox_created' }]);
  const s = await sources(t);
  const result = await storeClaudeProfile({ rpc: k.rpc, label: 'lanes-claude-old', token: 'sk-old', ...s });
  assert.equal(result.claudeProfile, 'lanes-claude-old-new1');
  assert.equal(result.created, true);
  assert.deepEqual(k.calls.map(c => Object.keys(c)[0]), ['ListProviderAccountProfiles', 'CreateProviderAccountProfile', 'ListProviderAccountProfiles', 'SetProviderAccountCredential', 'GetProviderAuthStatus']);
  assert.deepEqual(k.calls.find(c => c.SetProviderAccountCredential), { SetProviderAccountCredential: { provider: 'claude', account_profile: 'lanes-claude-old-new1', value: 'sk-old', overwrite: true } });
  assert.equal(await readFile(join(s.dir, 'config/lanes-claude-old-new1/agents/scout.md'), 'utf8'), 'scout definition');
  assert.ok(!JSON.stringify(result).includes('sk-old'));
});

test('an existing label is updated in place: no second profile, the default profile untouched', async t => {
  const k = kernel([{ label: 'lanes-claude', profile_id: 'lanes-claude-aaaa', origin: 'chariox_created' }, { label: 'lanes-claude-old', profile_id: 'lanes-claude-old-bbbb', origin: 'chariox_created' }]);
  const result = await storeClaudeProfile({ rpc: k.rpc, label: 'lanes-claude-old', token: 'sk-2', ...await sources(t) });
  assert.equal(result.claudeProfile, 'lanes-claude-old-bbbb');
  assert.equal(result.created, false);
  assert.ok(!k.calls.some(c => c.CreateProviderAccountProfile));
  assert.deepEqual(k.calls.filter(c => c.SetProviderAccountCredential).map(c => c.SetProviderAccountCredential.account_profile), ['lanes-claude-old-bbbb']);
});

test('a label held by a linked profile or by two profiles is refused before any credential is stored', async t => {
  const s = await sources(t);
  const linked = kernel([{ label: 'lanes-claude-old', profile_id: 'x', origin: 'linked' }]);
  await assert.rejects(storeClaudeProfile({ rpc: linked.rpc, label: 'lanes-claude-old', token: 't', ...s }), /belongs to a linked profile/);
  const twin = kernel([{ label: 'lanes-claude-old', profile_id: 'a', origin: 'chariox_created' }, { label: 'lanes-claude-old', profile_id: 'b', origin: 'chariox_created' }]);
  await assert.rejects(storeClaudeProfile({ rpc: twin.rpc, label: 'lanes-claude-old', token: 't', ...s }), /2 profiles are labelled/);
  assert.ok(![...linked.calls, ...twin.calls].some(c => c.SetProviderAccountCredential));
});

test('a profile whose config directory does not exist yet says so instead of skipping the agents', async t => {
  const k = kernel([{ label: 'l', profile_id: 'l-1', origin: 'chariox_created' }]);
  const s = await sources(t);
  const result = await storeClaudeProfile({ rpc: k.rpc, label: 'l', token: 't', ...s, configDirectory: () => undefined });
  assert.deepEqual(result.agents, { reason: 'profile config directory not found' });
});

test('--profile needs --from, and unknown or repeated arguments are refused', () => {
  assert.deepEqual(parseCredentialArgs([]), {});
  assert.deepEqual(parseCredentialArgs(['--from', '/t']), { from: '/t' });
  assert.deepEqual(parseCredentialArgs(['--profile', 'lanes-claude-old', '--from', '/t']), { profile: 'lanes-claude-old', from: '/t' });
  for (const args of [['--profile', 'lanes-claude-old'], ['--profile'], ['--from'], ['--profile', '../x', '--from', '/t'], ['--from', '/t', '--from', '/u'], ['--other', 'x'], ['/t']]) {
    assert.ok(parseCredentialArgs(args).error, JSON.stringify(args));
  }
});
