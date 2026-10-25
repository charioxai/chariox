// MP-08 / MP-09 / MP-10 / MP-11: the Mac's loopbrief.sh against a local stand-in for b3.sh and loopd.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile, readdir, mkdir, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

const script = new URL('./loopbrief.sh', import.meta.url).pathname;
async function rig(t) {
  const root = await mkdtemp(join(tmpdir(), 'loopbrief-')); t.after(() => rm(root, { recursive: true, force: true }));
  const shim = join(root, 'b3.sh');
  await writeFile(shim, '#!/bin/bash\nexec bash -c "$1"\n'); await chmod(shim, 0o755);
  const agents = join(root, 'agents');
  const run = (args, env, brief = 'the round brief') => {
    const child = spawn('bash', [script, ...args], { env: { PATH: process.env.PATH, HOME: root, B3: shim, LOOP_AGENTS: agents, WAIT_SECONDS: '6', ...env }, stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    child.stdout.on('data', c => { stdout += c; }); child.stderr.on('data', c => { stderr += c; });
    // Argument rejection may close stdin before the unused brief is written.
    child.stdin.on('error', error => { if (error.code !== 'EPIPE') child.emit('error', error); });
    child.stdin.end(brief);
    return { done: once(child, 'exit').then(([code]) => ({ code, stdout, stderr })) };
  };
  const waitFor = async path => { for (let i = 0; i < 100; i++) { try { return await readFile(path, 'utf8'); } catch { await new Promise(r => setTimeout(r, 100)); } } throw new Error(`no ${path}`); };
  return { root, agents, run, waitFor };
}

test('loopbrief writes the brief and a queue entry, then reports the run file once loopd picked it up', async t => {
  const r = await rig(t);
  const { done } = r.run(['integ2', '/work/integ2', 'codex'], { EFFORT: 'medium', HARD_STOP_MIN: '90', EXTRA_WORKTREES: '/work/cloud' });
  const entry = JSON.parse(await r.waitFor(join(r.agents, '_loop/queue/integ2-r1.json')));
  assert.deepEqual(entry, { lane: 'integ2', round: 'r1', provider: 'codex', model: 'gpt-6.1-sol', effort: 'medium', hardStopMinutes: 90, brief: join(r.agents, 'integ2/BRIEF-r1.md'), worktrees: ['/work/integ2', '/work/cloud'] });
  assert.equal(await readFile(entry.brief, 'utf8'), 'the round brief');
  await writeFile(join(r.agents, 'integ2/LOOP_RUN.json'), JSON.stringify({ round: 'r1', launchedAtMs: 1 }));
  const result = await done;
  assert.equal(result.code, 0, result.stderr);
  assert.match(result.stdout, /queue file: .*integ2-r1\.json\nrun file: +.*integ2\/LOOP_RUN\.json\npicked up by loopd as integ2-r1 \(gpt-6\.1-sol medium, hard stop 90 min\)/);
});

test('loopbrief names the shared rules as the entry preamble: the PREAMBLE env, else agents/common.md and agents/worker-session.md when they exist', async t => {
  const r = await rig(t);
  await mkdir(r.agents, { recursive: true });
  await writeFile(join(r.agents, 'common.md'), 'rules');
  await writeFile(join(r.agents, 'worker-session.md'), 'session rules');
  const first = r.run(['alpha', '/work/alpha', 'claude']);
  const entry = JSON.parse(await r.waitFor(join(r.agents, '_loop/queue/alpha-r1.json')));
  assert.deepEqual(entry.preamble, [join(r.agents, 'common.md'), join(r.agents, 'worker-session.md')]);
  await writeFile(join(r.agents, 'alpha/LOOP_RUN.json'), JSON.stringify({ round: 'r1', launchedAtMs: 1 }));
  assert.equal((await first.done).code, 0);
  const second = r.run(['beta', '/work/beta', 'claude'], { PREAMBLE: '/rules/a.md /rules/b.md' });
  assert.deepEqual(JSON.parse(await r.waitFor(join(r.agents, '_loop/queue/beta-r1.json'))).preamble, ['/rules/a.md', '/rules/b.md']);
  await writeFile(join(r.agents, 'beta/LOOP_RUN.json'), JSON.stringify({ round: 'r1', launchedAtMs: 1 }));
  assert.equal((await second.done).code, 0);
});

test('loopbrief numbers the next round after the briefs and queue entries it finds', async t => {
  const r = await rig(t);
  await mkdir(join(r.agents, 'amint'), { recursive: true });
  await mkdir(join(r.agents, '_loop/queue/done'), { recursive: true });
  await writeFile(join(r.agents, 'amint/BRIEF-r19.md'), 'old');
  await writeFile(join(r.agents, '_loop/queue/done/amint-r20.json'), '{}');
  const { done } = r.run(['amint', '/work/amint', 'claude'], { MODEL: 'claude-opus-5-5' });
  const entry = JSON.parse(await r.waitFor(join(r.agents, '_loop/queue/amint-r21.json')));
  assert.equal(entry.model, 'claude-opus-5-5');
  assert.equal(entry.effort, 'high');
  assert.equal(entry.hardStopMinutes, 240);
  await writeFile(join(r.agents, 'amint/LOOP_RUN.json'), JSON.stringify({ round: 'r21', launchedAtMs: 1 }));
  assert.equal((await done).code, 0);
});

test('loopbrief reports a rejected entry with loopd reason, and a loopd that never answers', async t => {
  const r = await rig(t);
  const { done } = r.run(['bad', '/work/bad', 'claude']);
  await r.waitFor(join(r.agents, '_loop/queue/bad-r1.json'));
  await mkdir(join(r.agents, '_loop/queue/rejected'), { recursive: true });
  await writeFile(join(r.agents, '_loop/queue/rejected/bad-r1.json'), '{}');
  await writeFile(join(r.agents, '_loop/queue/rejected/bad-r1.json.reason.txt'), 'invalid_queue_entry: effort: must be one of low');
  const rejected = await done;
  assert.equal(rejected.code, 3);
  assert.match(rejected.stderr, /effort: must be one of low/);
  const silent = await r.run(['quiet', '/work/quiet', 'claude'], { WAIT_SECONDS: '2' }).done;
  assert.equal(silent.code, 4);
  assert.match(silent.stderr, /loopd has not picked up quiet-r1/);
});

test('loopbrief refuses bad input before touching b3', async t => {
  const r = await rig(t);
  for (const [args, env] of [[['../x', '/w', 'claude'], {}], [['ok', 'relative', 'claude'], {}], [['ok', '/w', 'gemini'], {}], [['ok', '/w', 'claude'], { HARD_STOP_MIN: '0' }]]) {
    const result = await r.run(args, env).done;
    assert.equal(result.code, 2, JSON.stringify(args));
  }
  assert.deepEqual(await readdir(r.root).then(files => files.filter(f => f === 'agents')), []);
});
