// MP-08 / MP-09 / MP-10 / MP-11: capability-gated steer confirmation, no kernel fixtures.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, readFile, readdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { coordinate, save } from './loop.mjs';

const target = { kernel: 'executor', session_id: 'room', agent_id: 'agent' };
const event = { ...target, type: 'steer', value: { steer_id: 'provider-steer' }, cursor: 2 };
async function rig(t, capabilities = ['outside_steer_events_v1']) {
  const home = await mkdtemp(join(tmpdir(), 'delivery-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  await save(join(home, 'lane.json'), {
    lane: 'lane', input: { stateFile: join(home, 'lane.launch.json') },
    launch: { kernel: target.kernel, sessionId: target.session_id, agentId: target.agent_id, taskId: 'lane', attachmentId: 'attached' },
  });
  const review = join(home, 'review.md'); await writeFile(review, 'MP-11: review this seam');
  const calls = [];
  const options = { home, now: () => 1000, adapter: async (op, input) => {
    calls.push({ op, input });
    if (op === 'status') return { currentPrompt: 'current', state: 'working', capabilities };
    assert.equal(op, 'steer');
    return { ActivePromptSteered: { prompt: { id: 'provider-steer' } } };
  }, cli: async argv => {
    calls.push({ argv });
    if (argv[1] === 'subscription') return { subscription: { subscription_id: 'grant', targets: [target] } };
    if (argv[1] === 'wait') return { events: [event], next_cursor: event.cursor, timed_out: false };
    assert.equal(argv[1], 'ack');
    assert.ok((await readdir(join(home, 'events'))).length);
    const receipt = JSON.parse(await readFile(join(home, 'lane.delivery.json'), 'utf8'));
    assert.equal(receipt.result.confirmation.mode, 'event', 'confirmation is durable before the matching event is acknowledged');
    return {};
  } };
  return { home, options, calls, deliver: () => coordinate('deliver', ['lane', review], options) };
}

test('deliver waits for the exact returned steer id and target, archiving before ack', async t => {
  const r = await rig(t); const cli = r.options.cli; let waits = 0;
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && waits++ === 0) {
      r.calls.push({ argv });
      return { events: [
        { ...event, kernel: 'other' }, { ...event, session_id: 'other' },
        { ...event, agent_id: 'other' }, { ...event, value: { steer_id: 'current' } },
      ].map((value, index) => ({ ...value, cursor: index + 1 })), next_cursor: 4 };
    }
    if (argv[1] === 'ack' && argv[2] === '4') {
      r.calls.push({ argv });
      assert.equal((await readdir(join(r.home, 'events'))).length, 1);
      return {};
    }
    if (argv[1] === 'wait') { r.calls.push({ argv }); return { events: [{ ...event, cursor: 5 }], next_cursor: 5 }; }
    return cli(argv);
  };
  const result = await r.deliver();
  assert.equal(result.confirmation.mode, 'event');
  assert.deepEqual(result.confirmation.event, { ...event, cursor: 5 });
  assert.equal(result.confirmation.archives.length, 2);
  assert.deepEqual(r.calls.map(c => c.op || c.argv[1]), ['status', 'subscription', 'steer', 'wait', 'ack', 'wait', 'ack']);
  assert.equal(r.calls.find(c => c.op === 'steer').input.expectedPromptId, 'current');
});

test('deliver without the capability reads status after the acknowledged steer', async t => {
  const r = await rig(t, []);
  r.options.cli = () => assert.fail('older kernels must not be asked for a steer event');
  const result = await r.deliver();
  assert.equal(result.confirmation.mode, 'acknowledged-steer-status');
  assert.equal(result.confirmation.status.currentPrompt, 'current');
  assert.deepEqual(r.calls.map(c => c.op), ['status', 'steer', 'status']);
  assert.deepEqual(JSON.parse(await readFile(join(r.home, 'lane.delivery.json'), 'utf8')).result, result);
});

for (const type of ['gap', 'subscription_ended']) test(`deliver refuses ${type} without ack or status fallback`, async t => {
  const r = await rig(t); const cli = r.options.cli;
  const batch = { events: [{ type, value: { reason: 'grant_revoked' } }, event], next_cursor: 3 };
  r.options.cli = argv => argv[1] === 'wait' ? batch : cli(argv);
  await assert.rejects(r.deliver(), { code: 'event_stream_incomplete' });
  assert.equal(r.calls.some(c => c.argv?.[1] === 'ack'), false);
  assert.equal(r.calls.filter(c => c.op === 'status').length, 1);
  const files = await readdir(join(r.home, 'events'));
  assert.deepEqual(JSON.parse(await readFile(join(r.home, 'events', files[0]), 'utf8')), batch);
});

test('deliver times out explicitly without claiming event confirmation', async t => {
  const r = await rig(t); const cli = r.options.cli;
  r.options.cli = argv => argv[1] === 'wait' ? { events: [], next_cursor: 0, timed_out: true } : cli(argv);
  await assert.rejects(r.deliver(), { code: 'steer_confirmation_timeout' });
  assert.equal(r.calls.some(c => c.argv?.[1] === 'ack'), false);
  assert.equal(JSON.parse(await readFile(join(r.home, 'lane.delivery.json'), 'utf8')).result.confirmation, undefined);
});

test('deliver validates the subscription before sending the steer', async t => {
  const r = await rig(t);
  r.options.cli = async () => ({ subscription: { subscription_id: 'grant', targets: [] } });
  await assert.rejects(r.deliver(), { code: 'subscription_target_missing' });
  assert.equal(r.calls.some(c => c.op === 'steer'), false);
});

test('deliver refuses a response without a returned steer id', async t => {
  const r = await rig(t, []); const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'steer' ? {} : adapter(op, input);
  await assert.rejects(r.deliver(), { code: 'unexpected_steer_response' });
  assert.equal(r.calls.filter(c => c.op === 'status').length, 1);
});

test('deliver resumes an interrupted confirmation and reuses its durable receipt', async t => {
  const r = await rig(t); const cli = r.options.cli; let fail = true;
  r.options.cli = async argv => {
    if (argv[1] === 'wait' && fail) { fail = false; throw Object.assign(new Error('disconnected'), { code: 'kernel_connection_lost' }); }
    return cli(argv);
  };
  await assert.rejects(r.deliver(), { code: 'kernel_connection_lost' });
  const adapter = r.options.adapter;
  r.options.adapter = async (op, input) => op === 'status' ? { ...await adapter(op, input), currentPrompt: null } : adapter(op, input);
  const result = await r.deliver();
  assert.equal(result.confirmation.mode, 'event');
  assert.deepEqual(await r.deliver(), result);
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1);
  assert.equal(r.calls.filter(c => c.argv?.[1] === 'ack').length, 1);
});

test('deliver preserves event confirmation when its advertised capability disappears', async t => {
  const capabilities = ['outside_steer_events_v1'];
  const r = await rig(t, capabilities); const cli = r.options.cli;
  r.options.cli = argv => argv[1] === 'wait' ? { events: [], next_cursor: 0, timed_out: true } : cli(argv);
  await assert.rejects(r.deliver(), { code: 'steer_confirmation_timeout' });
  capabilities.length = 0;
  await assert.rejects(r.deliver(), { code: 'steer_events_unavailable' });
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1);
});

test('deliver retains a confirmed receipt if acknowledging its event fails', async t => {
  const r = await rig(t); const cli = r.options.cli;
  r.options.cli = async argv => {
    const result = await cli(argv);
    if (argv[1] === 'ack') throw Object.assign(new Error('disconnected'), { code: 'kernel_connection_lost' });
    return result;
  };
  await assert.rejects(r.deliver(), { code: 'kernel_connection_lost' });
  assert.equal((await r.deliver()).confirmation.mode, 'event');
  assert.equal(r.calls.filter(c => c.op === 'steer').length, 1);
});
