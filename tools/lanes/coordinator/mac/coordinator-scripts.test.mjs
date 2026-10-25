// MP-08 / MP-09 / MP-10 / MP-11: the Mac's lanewait.sh, deliver.sh and laneanswer.sh against a stand-in b3.sh and a stand-in lanes tree.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile, mkdir, chmod } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';

const here = name => new URL(`./${name}`, import.meta.url).pathname;
async function rig(t, steerBody = "console.log(JSON.stringify({ lane: process.argv[2], mode: 'steered' }));") {
  const root = await mkdtemp(join(tmpdir(), 'coordscripts-')); t.after(() => rm(root, { recursive: true, force: true }));
  const state = join(root, 'state'), agents = join(root, 'agents'), run = join(state, 'run/tools/lanes');
  await mkdir(join(run, 'kernel'), { recursive: true }); await mkdir(join(run, 'coordinator'), { recursive: true }); await mkdir(join(agents, 'alpha'), { recursive: true });
  const shim = join(root, 'b3.sh'); await writeFile(shim, '#!/bin/bash\nexec bash -c "$1"\n'); await chmod(shim, 0o755);
  // exec --seat <seat> -- <command...>: records the seat, then runs the command like the holder would
  await writeFile(join(run, 'kernel/lanes-kernel.mjs'), `import { spawnSync } from 'node:child_process'; import { appendFileSync } from 'node:fs';
    const a = process.argv.slice(2); const seat = a[a.indexOf('--seat') + 1]; const rest = a.slice(a.indexOf('--') + 1);
    appendFileSync(${JSON.stringify(join(root, 'calls.log'))}, JSON.stringify({ seat, env: process.env.LANES_KERNEL_STATE, agents: process.env.LOOPD_AGENTS, rest: rest.slice(1).map(x => x.replace(/^.*\\//, '')) }) + '\\n');
    const r = spawnSync(rest[0], rest.slice(1), { stdio: 'inherit' }); process.exitCode = r.status;`);
  await writeFile(join(run, 'coordinator/steer.mjs'), `import { readFileSync, existsSync } from 'node:fs';
    ${steerBody}
    `);
  await writeFile(join(run, 'coordinator/answer.mjs'), "console.log(JSON.stringify({ answered: process.argv.slice(2) }));");
  await writeFile(join(run, 'coordinator/lanewait.mjs'), "console.log(process.argv.slice(2).join(' ')); process.exitCode = Number(process.env.WAIT_EXIT || 0);");
  const env = { PATH: process.env.PATH, HOME: root, B3: shim, LANES_STATE: state, LOOP_AGENTS: agents };
  const sh = (script, args, input = '', extra = {}) => spawnSync('bash', [here(script), ...args], { env: { ...env, ...extra }, input, encoding: 'utf8' });
  const calls = async () => (await readFile(join(root, 'calls.log'), 'utf8')).trim().split('\n').map(line => JSON.parse(line));
  return { root, agents, sh, calls };
}

test('lanewait.sh runs the wait beneath the coordinator seat with its own state, and passes the exit code through', async t => {
  const r = await rig(t);
  const result = r.sh('lanewait.sh', ['90', '--json']);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout.trim(), '--minutes 90 --json');
  const [call] = await r.calls();
  assert.equal(call.seat, 'coordinator');
  assert.equal(call.agents, r.agents);
  const quiet = r.sh('lanewait.sh', ['5'], '', { WAIT_EXIT: '3' });
  assert.equal(quiet.status, 3);
  assert.match(quiet.stderr, /no worker events/);
  assert.equal(r.sh('lanewait.sh', ['--replay', '5']).stdout.trim(), '--replay 5');
  assert.equal(r.sh('lanewait.sh', ['5000']).status, 2);
});

test('deliver.sh steers first, then appends the section (with a heading and a final newline) to the lane inbox', async t => {
  const r = await rig(t, `const text = readFileSync(process.argv[3], 'utf8');
    const inbox = readFileSync(process.env.LOOPD_AGENTS + '/' + process.argv[2] + '/REVIEW_INBOX.md', 'utf8');
    console.log(JSON.stringify({ lane: process.argv[2], text, inboxHadTextAtSteer: inbox.includes(text) }));`);
  await writeFile(join(r.agents, 'alpha/REVIEW_INBOX.md'), '## old\nearlier text without newline');
  const result = r.sh('deliver.sh', ['alpha', '-'], 'please fix X');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /steered alpha directly: \{"lane":"alpha","text":"## \d\d:\d\dZ coordinator\\nplease fix X\\n","inboxHadTextAtSteer":false\}/);
  const inbox = await readFile(join(r.agents, 'alpha/REVIEW_INBOX.md'), 'utf8');
  assert.match(inbox, /^## old\nearlier text without newline\n## \d\d:\d\dZ coordinator\nplease fix X\n$/);
  assert.equal((await r.calls())[0].seat, 'coordinator');
});

test('deliver.sh: a lane with no round under loopd only gets the append; a failed steer appends, says so and exits 4', async t => {
  const classic = await rig(t, "console.error(JSON.stringify({ error: { code: 'lane_not_running', message: 'no round' } })); process.exitCode = 1;");
  const first = classic.sh('deliver.sh', ['alpha', '-'], '## 10:00Z coordinator\nnote\n');
  assert.equal(first.status, 0);
  assert.match(first.stdout, /classic lane/);
  assert.equal(await readFile(join(classic.agents, 'alpha/REVIEW_INBOX.md'), 'utf8'), '## 10:00Z coordinator\nnote\n');
  const broken = await rig(t, "console.error(JSON.stringify({ error: { code: 'kernel_connection_lost' } })); process.exitCode = 1;");
  const second = broken.sh('deliver.sh', ['alpha', '-'], '## 10:00Z coordinator\nnote\n');
  assert.equal(second.status, 4);
  assert.match(second.stderr, /kernel_connection_lost/);
  assert.match(second.stderr, /loopd delivers it at its next wake/);
  assert.equal(await readFile(join(broken.agents, 'alpha/REVIEW_INBOX.md'), 'utf8'), '## 10:00Z coordinator\nnote\n');
});

test('deliver.sh refuses an empty text and a bad lane name before touching b3', async t => {
  const r = await rig(t);
  assert.equal(r.sh('deliver.sh', ['alpha', '-'], '  \n').status, 2);
  assert.equal(r.sh('deliver.sh', ['bad lane', '-'], 'x').status, 2);
  assert.equal(existsSync(join(r.root, 'calls.log')), false);
});

test('laneanswer.sh passes the lane, decision and choice to the coordinator seat', async t => {
  const r = await rig(t);
  const result = r.sh('laneanswer.sh', ['alpha', 'ask-1', 'blue']);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { answered: ['alpha', 'ask-1', 'blue'] });
  assert.equal((await r.calls())[0].seat, 'coordinator');
  assert.equal(r.sh('laneanswer.sh', ['alpha', 'a b', 'blue']).status, 2);
});

// switch-claude-account.sh against a stand-in adapters tree (probe and kernel command are stubs) and a token directory.
async function switchRig(t) {
  const root = await mkdtemp(join(tmpdir(), 'switchacct-')); t.after(() => rm(root, { recursive: true, force: true }));
  const kernel = join(root, 'adapters/tools/lanes/kernel'), tokens = join(root, 'tokens'), agents = join(root, 'agents');
  await mkdir(kernel, { recursive: true }); await mkdir(tokens); await mkdir(agents);
  await writeFile(join(tokens, 'oauth-token'), 'ACTIVE\n'); await writeFile(join(tokens, 'oauth-token.chariox'), 'CHARIOX\n'); await writeFile(join(tokens, 'oauth-token.old-account'), 'OLD\n');
  await writeFile(join(kernel, 'claude-account.mjs'), `import { readFileSync } from 'node:fs';
    const usable = readFileSync(process.argv[3], 'utf8').trim() !== 'EXHAUSTED';
    console.log(JSON.stringify({ usable, org: '0123456789ab' })); process.exitCode = usable ? 0 : 3;`);
  await writeFile(join(kernel, 'lanes-kernel.mjs'), `import { appendFileSync } from 'node:fs';
    appendFileSync(${JSON.stringify(join(root, 'kernel.log'))}, JSON.stringify({ args: process.argv.slice(2), state: process.env.LANES_KERNEL_STATE }) + '\\n');
    if (process.env.KERNEL_REFUSES) { console.error('{"error":"vault locked"}'); process.exitCode = 1; } else console.log(JSON.stringify({ auth: ['authenticated'], org: '0123456789ab' }));`);
  const run = (target, extra = {}) => spawnSync('bash', [here('switch-claude-account.sh'), ...target], { env: { PATH: process.env.PATH, HOME: root, B3: 'local', LANES_STATE: join(root, 'state'), LOOP_AGENTS: agents, CLAUDE_TOKENS: tokens, ADAPTERS: join(root, 'adapters'), ...extra }, encoding: 'utf8' });
  return { root, tokens, agents, run, active: async () => (await readFile(join(tokens, 'oauth-token'), 'utf8')).trim(), kernelCalls: async () => existsSync(join(root, 'kernel.log')) ? (await readFile(join(root, 'kernel.log'), 'utf8')).trim().split('\n').map(l => JSON.parse(l)) : [] };
}

test('switch-claude-account.sh re-stores the kernel credential from the target file and then makes it the active token', async t => {
  const r = await switchRig(t);
  const result = r.run(['chariox']);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(await r.active(), 'CHARIOX');
  const [call] = await r.kernelCalls();
  assert.deepEqual(call.args, ['set-claude-credential', '--from', join(r.tokens, 'oauth-token.chariox')]);
  assert.equal(call.state, join(r.root, 'state'));
  assert.ok(existsSync(join(r.agents, 'CLAUDE_SWITCHED')));
  assert.ok(!result.stdout.includes('CHARIOX') && !result.stderr.includes('CHARIOX'), 'a token must never be printed');
  assert.equal((await readFile(join(r.tokens, 'oauth-token.old-account'), 'utf8')).trim(), 'OLD');
});

test('switch-claude-account.sh refuses an account that is not usable yet and changes nothing', async t => {
  const r = await switchRig(t);
  await writeFile(join(r.tokens, 'oauth-token.chariox'), 'EXHAUSTED\n');
  const result = r.run(['chariox']);
  assert.equal(result.status, 3);
  assert.match(result.stderr, /not usable yet; nothing changed/);
  assert.equal(await r.active(), 'ACTIVE');
  assert.deepEqual(await r.kernelCalls(), []);
});

test('switch-claude-account.sh leaves the active token alone when the kernel refuses, and rejects an unknown target', async t => {
  const r = await switchRig(t);
  const refused = r.run(['old'], { KERNEL_REFUSES: '1' });
  assert.equal(refused.status, 5);
  assert.equal(await r.active(), 'ACTIVE');
  assert.equal(r.run(['elsewhere']).status, 2);
  assert.equal(r.run([]).status, 2);
});

test('switch reports a partial change when staging, rename or marker fails after kernel success', async t => {
  for (const failure of ['copy', 'rename', 'marker']) {
    const r = await switchRig(t);
    // A file used as a parent fails even as root, without relying on chmod.
    if (failure === 'copy') {
      await mkdir(join(r.tokens, '.oauth-token.new'));
      await mkdir(join(r.tokens, '.oauth-token.new/oauth-token.chariox'));
    }
    if (failure === 'rename') await mkdir(join(r.tokens, 'oauth-token/child'), { recursive: true }).catch(async () => {
      await rm(join(r.tokens, 'oauth-token')); await mkdir(join(r.tokens, 'oauth-token/child'), { recursive: true });
    });
    if (failure === 'marker') await writeFile(join(r.root, 'not-a-directory'), 'fixture');
    const result = r.run(['chariox'], failure === 'marker' ? { LOOP_AGENTS: join(r.root, 'not-a-directory') } : {});
    assert.notEqual(result.status, 0, `${failure}: cannot report a complete switch`);
    assert.match(result.stderr, /kernel credential has already changed/);
    assert.equal((await r.kernelCalls()).length, 1);
    assert.equal(result.stdout.includes('oauth-token now holds'), false);
    assert.equal(existsSync(join(r.agents, 'CLAUDE_SWITCHED')), false);
  }
});

test('switch probe-only checks usability and never updates the kernel, active file or marker', async t => {
  const r = await switchRig(t);
  assert.equal(r.run(['chariox', '--probe']).status, 0);
  assert.deepEqual(await r.kernelCalls(), []);
  assert.equal(await r.active(), 'ACTIVE');
  assert.equal(existsSync(join(r.agents, 'CLAUDE_SWITCHED')), false);
  await writeFile(join(r.tokens, 'oauth-token.chariox'), 'EXHAUSTED\n');
  assert.equal(r.run(['chariox', '--probe']).status, 3);
  assert.deepEqual(await r.kernelCalls(), []);
});
