// MP-08 / MP-10: acceptance receipts may already be consumed by the coordinator.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, rm, copyFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, basename } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { run, LaneError } from './lanes.mjs';
const common = { kernel: 'executor', taskId: 'lane', sessionId: 'ops' };
async function scratch(t) { const p = await mkdtemp(join(tmpdir(), 'lane-collection-')); t.after(() => rm(p, { recursive: true, force: true })); return p; }
test('fetch reports consumed acceptance JSON and keeps the required handoffs', async t => {
  const directory = await scratch(t);
  const result = await run('fetch', { ...common, worktree: '/lane-state', directory }, { execute: async argv => {
    if (argv.at(-2).endsWith('/ACCEPTANCE_UPDATE.json')) throw new LaneError('file_not_found');
    await writeFile(argv.at(-1), 'MP-08 / MP-10 handoff'); return {};
  } });
  assert.deepEqual(result['ACCEPTANCE_UPDATE.json'], { missing: true, reason: 'file_not_found' });
  assert.equal(await readFile(result['PUSH_READY.md'].path, 'utf8'), 'MP-08 / MP-10 handoff');
});
test('fetch propagates authorization failures, reports a missing handoff and acceptance problems', async t => {
  const directory = await scratch(t);
  for (const [file, code] of [['LANE_STATUS.md', 'kernel_access_required'], ['ACCEPTANCE_UPDATE.json', 'kernel_access_required']]) {
    await assert.rejects(run('fetch', { ...common, worktree: '/lane-state', directory }, { execute: async argv => {
      if (basename(argv.at(-2)) === file) throw new LaneError(code);
      await writeFile(argv.at(-1), 'handoff'); return {};
    } }), { code });
  }
  const stopped = await run('fetch', { ...common, worktree: '/lane-state', directory }, { execute: async argv => {
    if (basename(argv.at(-2)) === 'LANE_STATUS.md') throw new LaneError('file_not_found');
    await writeFile(argv.at(-1), 'handoff'); return {};
  } });
  assert.deepEqual(stopped['LANE_STATUS.md'], { missing: true, reason: 'file_not_found' });
  const files = await run('fetch', { ...common, worktree: '/lane-state', directory }, { execute: async argv => {
    await writeFile(argv.at(-1), argv.at(-2).endsWith('.json') ? '{"rows":[{}]}' : 'handoff'); return {};
  } });
  assert.equal(files['ACCEPTANCE_UPDATE.json'].acceptance.valid, false);
});
async function bundleRig(t) {
  const dir = await scratch(t), source = join(dir, 'source'), repository = join(dir, 'imported');
  const git = argv => execFileSync('git', argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  git(['init', '-b', 'main', source]); git(['init', '--bare', repository]);
  await writeFile(join(source, 'source.txt'), 'base'); git(['-C', source, 'add', 'source.txt']);
  git(['-C', source, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-m', 'Create fixture [skip ci]']);
  const base = git(['-C', source, 'rev-parse', 'HEAD']).trim();
  await writeFile(join(source, 'source.txt'), 'head'); git(['-C', source, 'add', 'source.txt']);
  git(['-C', source, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-m', 'Update fixture [skip ci]']);
  const head = git(['-C', source, 'rev-parse', 'HEAD']).trim();
  const input = { ...common, base, worktree: source, remoteBundle: join(dir, 'remote.bundle'), localBundle: join(dir, 'local.bundle'), repository };
  let screenText = '';
  const execute = async (argv, body) => {
    if (argv[0] === 'terminal') {
      if (argv[1] === 'open') { const command = argv.slice(argv.indexOf('--') + 1); screenText = execFileSync(command[0], command.slice(1), { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }); return { terminal: { terminal_id: 'owned' } }; }
      if (argv[1] === 'wait') return { fired: { kind: 'finished', exit_code: 0 }, screen_text: screenText };
      return {};
    }
    if (argv[0] === 'files') {
      assert.equal(argv[argv.indexOf('--sha256') + 1], createHash('sha256').update(await readFile(input.remoteBundle)).digest('hex'));
      await copyFile(argv.at(-2), argv.at(-1)); return {};
    }
    assert.fail(`unexpected request: ${body}`);
  };
  return { input, head, git, execute };
}
test('bundle resolves actual HEAD from a real bundle without an acceptance receipt', async t => {
  const r = await bundleRig(t); const out = await run('bundle', r.input, r);
  assert.equal(out.head, r.head);
  assert.equal(r.git(['-C', r.input.repository, 'rev-parse', 'refs/remotes/lanes/lane']).trim(), r.head);
});

for (const corruption of [undefined, 'stat', 'download']) {
  test(`bundle falls back only for an older CLI usage refusal (${corruption ?? 'verified'})`, async t => {
    const r = await bundleRig(t), calls = [];
    const execute = async (argv, body) => {
      if (argv[0] === 'request') {
        calls.push('stat');
        assert.deepEqual(JSON.parse(body), { StatKernelFile: { path: r.input.remoteBundle, with_sha256: true } });
        const sha256 = corruption === 'stat' ? '0'.repeat(64) : createHash('sha256').update(await readFile(r.input.remoteBundle)).digest('hex');
        return { KernelFileStat: { stat: { sha256 } } };
      }
      if (argv[0] === 'files') {
        if (argv.includes('--sha256')) { calls.push('sha256'); throw new LaneError('usage'); }
        calls.push('download');
        await copyFile(r.input.remoteBundle, r.input.localBundle);
        if (corruption === 'download') await writeFile(r.input.localBundle, 'changed in transit');
        return {};
      }
      return r.execute(argv, body);
    };
    if (corruption) {
      await assert.rejects(run('bundle', r.input, { ...r, execute }), { code: 'bundle_hash_mismatch' });
      assert.throws(() => r.git(['-C', r.input.repository, 'show-ref', '--verify', 'refs/remotes/lanes/lane']));
    } else {
      assert.equal((await run('bundle', r.input, { ...r, execute })).head, r.head);
      assert.equal(r.git(['-C', r.input.repository, 'rev-parse', 'refs/remotes/lanes/lane']).trim(), r.head);
    }
    assert.deepEqual(calls, corruption === 'stat' ? ['sha256', 'stat'] : ['sha256', 'stat', 'download']);
  });
}

test('bundle propagates download authorization errors without a compatibility retry', async t => {
  const r = await bundleRig(t);
  const execute = async (argv, body) => {
    if (argv[0] === 'files') throw new LaneError('kernel_access_required');
    return r.execute(argv, body);
  };
  await assert.rejects(run('bundle', r.input, { ...r, execute }), { code: 'kernel_access_required' });
});
test('bundle rejects absent, non-HEAD and ambiguous tips before importing', async t => {
  const r = await bundleRig(t);
  for (const tips of ['', `${r.head} refs/heads/main\n`, `${r.head} HEAD\n${r.head} refs/heads/other\n`]) {
    const git = argv => argv[0] === 'bundle' && argv[1] === 'list-heads' ? tips : r.git(argv);
    await assert.rejects(run('bundle', r.input, { ...r, git }), { code: 'bundle_head_mismatch' });
    assert.throws(() => r.git(['-C', r.input.repository, 'show-ref', '--verify', 'refs/remotes/lanes/lane']));
  }
});

test('bundle keeps the expected-head guard after transfer', async t => {
  const r = await bundleRig(t);
  const git = argv => argv[0] === 'bundle' && argv[1] === 'list-heads' ? `${'b'.repeat(40)} HEAD\n` : r.git(argv);
  await assert.rejects(run('bundle', { ...r.input, head: r.head }, { ...r, git }), { code: 'bundle_head_mismatch' });
  assert.throws(() => r.git(['-C', r.input.repository, 'show-ref', '--verify', 'refs/remotes/lanes/lane']));
});
test('bundle removes its temporary ref when local ancestry verification fails', async t => {
  const r = await bundleRig(t);
  const git = argv => { if (argv[2] === 'merge-base') throw new Error('base is not an ancestor'); return r.git(argv); };
  await assert.rejects(run('bundle', r.input, { ...r, git }), /base is not an ancestor/);
  for (const ref of ['refs/lanes/incoming/lane', 'refs/remotes/lanes/lane']) assert.throws(() => r.git(['-C', r.input.repository, 'show-ref', '--verify', ref]));
});
