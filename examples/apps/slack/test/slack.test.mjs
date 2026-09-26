// Exercises the Slack backend against an in-memory stand-in for the kernel's
// App state (compare-and-set) and occurrence contracts, with deliveries shaped
// like the kernel's generator-fed inbox payload (protocol 358).
import assert from 'node:assert/strict';
import test from 'node:test';
import { occurrenceId } from '../../../../packages/app-sdk/src/occurrences.js';
import { AppError } from '../../../../packages/app-sdk/src/errors.js';
import register from '../bundle/runtime/main.mjs';

function fakeKernel({ automation = true } = {}) {
  const state = new Map();
  const occurrences = [];
  const actions = [];
  const tools = new Map();
  const incoming = new Map();
  const chariox = {
    AppError,
    tools: { register: (name, handler) => tools.set(name, handler) },
    events: { occurrenceId, register: (name, handler) => incoming.set(name, handler) },
    // Like the kernel: only a granted connection and a declared action pass.
    connections: {
      async action(request) {
        actions.push(request);
        return { accepted: true, result: { message_ts: '1.9' }, idempotencyKey: 'app:slack:k' };
      },
    },
    state: {
      async get(key) { return state.get(key) ?? null; },
      async transaction({ schemaVersion, checks, writes, occurrences: emitted = [] }) {
        if (schemaVersion !== 0) throw new AppError('SCHEMA_MISMATCH', 'App state schema version does not match');
        // Like the kernel, an occurrence for an unconfigured automation rolls
        // back the whole transaction.
        if (emitted.length && !automation) throw new AppError('NOT_FOUND', 'App automation was not found');
        for (const check of checks) {
          if ((state.get(check.key)?.version ?? null) !== check.version) {
            throw Object.assign(new Error('conflict'), { code: 'CONFLICT' });
          }
        }
        for (const write of writes) state.set(write.key, { value: write.value, version: (state.get(write.key)?.version ?? 0) + 1 });
        occurrences.push(...emitted);
        return { revision: 1, receipts: [] };
      },
    },
  };
  register(chariox);
  const deliver = (name, id, payload) => incoming.get(name)({ occurrenceId: id, payload });
  return { tools, occurrences, deliver, incoming, actions };
}

// As the Slack event generator forwards an app_mention through AEDS.
function mention(text, channel = 'C1', user = 'U1') {
  return {
    source: { generator_id: 'dev.chariox.slack', connection_id: 'connection-1', event_type: 'app.mentioned', event_type_version: 1 },
    occurred_at: '2026-09-26T19:00:00.000Z',
    text: `Handle Slack app.mentioned: ${text}`,
    metadata: { team_id: 'T1', event: { type: 'app_mention', text, channel, user, ts: '1.2' } },
    artifacts: [],
    reply_context: { provider: 'slack', team_id: 'T1', channel_id: channel, message_ts: '1.2', thread_ts: '1.2', user_id: user },
  };
}

test('every signed incoming Slack event has a handler', () => {
  const kernel = fakeKernel();
  assert.deepEqual([...kernel.incoming.keys()].sort(), ['channel_message', 'mentioned', 'reaction_added']);
});

test('a mention is kept once and forwarded once to the notifications automation', async () => {
  const kernel = fakeKernel();
  await kernel.deliver('mentioned', 'Ev1', mention('<@B1> deploy status?'));
  await kernel.deliver('mentioned', 'Ev1', mention('<@B1> deploy status?'));
  const { notifications } = await kernel.tools.get('list_notifications')({});
  assert.equal(notifications.length, 1, 'a redelivered event is stored once');
  assert.deepEqual(notifications[0], {
    id: 'Ev1', kind: 'mentioned', text: '<@B1> deploy status?', channel: 'C1', user: 'U1',
    occurred_at: '2026-09-26T19:00:00.000Z', can_reply: true,
  }, 'the reply context and connection stay inside the App');
  assert.equal(kernel.occurrences.length, 1);
  const [occurrence] = kernel.occurrences;
  assert.equal(occurrence.automationId, 'notifications');
  assert.equal(occurrence.occurrenceId, occurrenceId('slack:Ev1', Date.parse('2026-09-26T19:00:00.000Z')));
  assert.deepEqual(occurrence.payload, {
    kind: 'mentioned', text: '<@B1> deploy status?', occurred_at: '2026-09-26T19:00:00.000Z', channel: 'C1', user: 'U1',
  });
  assert.match(occurrence.invocation.prompt, /^Slack mentioned in <#C1> from <@U1>: <@B1> deploy status\?$/);
});

test('without a configured automation the notification is still kept', async () => {
  const kernel = fakeKernel({ automation: false });
  await kernel.deliver('channel_message', 'Ev2', { ...mention('hello'), source: { ...mention('').source, event_type: 'message.channels' } });
  assert.equal((await kernel.tools.get('list_notifications')({})).notifications.length, 1);
  assert.equal(kernel.occurrences.length, 0);
});

test('reactions read from the reacted-to item, and the list filters and clears', async () => {
  const kernel = fakeKernel();
  await kernel.deliver('mentioned', 'Ev1', mention('first'));
  await kernel.deliver('reaction_added', 'Ev3', {
    ...mention(''), metadata: { event: { type: 'reaction_added', reaction: 'tada', user: 'U2', item: { channel: 'C2', ts: '1.2' } } },
  });
  const reactions = (await kernel.tools.get('list_notifications')({ kind: 'reaction_added' })).notifications;
  assert.deepEqual(reactions.map(({ text, channel, user }) => ({ text, channel, user })),
    [{ text: ':tada: on a message', channel: 'C2', user: 'U2' }]);
  assert.equal((await kernel.tools.get('list_notifications')({})).notifications[0].id, 'Ev3', 'newest first');
  assert.deepEqual(await kernel.tools.get('clear_notifications')({}), { cleared: 2 });
  assert.equal((await kernel.tools.get('list_notifications')({})).notifications.length, 0);
});

test('a reply goes back through the notification\'s own connection and context', async () => {
  const kernel = fakeKernel();
  await kernel.deliver('mentioned', 'Ev1', mention('<@B1> deploy status?'));
  assert.deepEqual(await kernel.tools.get('reply')({ id: 'Ev1', text: 'Deploying now' }), { posted: true, message_ts: '1.9' });
  await kernel.tools.get('react')({ id: 'Ev1', name: 'eyes' });
  assert.deepEqual(kernel.actions, [
    { connectionId: 'connection-1', action: 'notification.reply', input: { text: 'Deploying now', mode: 'thread' },
      context: mention('').reply_context },
    { connectionId: 'connection-1', action: 'slack.reaction.add', input: { name: 'eyes' }, context: mention('').reply_context,
      idempotencyKey: 'react:Ev1:eyes' },
  ]);
  await assert.rejects(kernel.tools.get('reply')({ id: 'missing', text: 'x' }), { code: 'NOT_FOUND' });
  await kernel.deliver('mentioned', 'Ev9', { ...mention('no context'), reply_context: null });
  await assert.rejects(kernel.tools.get('reply')({ id: 'Ev9', text: 'x' }), { code: 'INVALID_ARGUMENT' });
});
