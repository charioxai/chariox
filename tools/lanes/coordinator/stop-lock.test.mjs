// MP-08 / MP-09 / MP-10 / MP-11: direct stop must share admission with attachment renewal.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { coordinate, save } from './loop.mjs';
import { LaneError } from '../lanes.mjs';

test('a direct stop waits for renewal and preserves the new attachment and admitted prompt', async t => {
  const home = await mkdtemp(join(tmpdir(), 'stop-renewal-')); t.after(() => rm(home, { recursive: true, force: true }));
  const launch = { kernel: 'executor', taskId: 'lane', sessionId: 'room', operationsSessionId: 'ops', agentId: 'agent', attachmentId: 'old' };
  await save(join(home, 'lane.json'), { lane: 'lane', launch, input: { stateFile: join(home, 'launch.json') } });
  const note = join(home, 'note.md'); await writeFile(note, 'MP-08 / MP-09 / MP-10 / MP-11: next prompt');
  let renewing, release;
  const started = new Promise(resolve => { renewing = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  const options = { home, adapter: async (op, input) => {
    if (op === 'status') return { currentPrompt: null };
    if (op === 'launch') { renewing(); await gate; return { ...launch, attachmentId: 'new' }; }
    if (op === 'prompt') {
      if (input.attachmentId === 'old') throw new LaneError('attachment_not_found');
      return { PromptSubmitted: { outcome: { Started: { prompt: { id: 'next', status: 'running' } } } } };
    }
    if (op === 'stop') return { stopped: true };
    throw new Error(`unexpected ${op}`);
  }, cli: async argv => {
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [] } };
    if (argv[1] === 'wait') return { events: [{ cursor: 1, type: 'session', kernel: 'executor', session_id: 'room', agent_id: 'agent', value: { state: 'agent_removed' } }], next_cursor: 1 };
    return {};
  } };
  const steer = coordinate('steer', ['lane', note], options);
  await started;
  const stop = coordinate('stop', ['lane'], options);
  await delay(100); release();
  await Promise.all([steer, stop]);
  const record = JSON.parse(await readFile(join(home, 'lane.json'), 'utf8'));
  assert.equal(record.stopped, true);
  assert.equal(record.launch.attachmentId, 'new');
  assert.equal(record.latestPromptId, 'next');
});
