import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createApp } from '../handler.mjs';

class AppError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}
const kinds = {
  pull_request_opened: 'pull_request.opened',
  pull_request_updated: 'pull_request.synchronize',
  review_requested: 'pull_request.review_requested',
  issue_opened: 'issues.opened',
  workflow_run_completed: 'workflow_run.completed',
};
function kernel(saved = new Map(), receipts = new Map()) {
  const incoming = new Map(), tools = new Map(), calls = [];
  let reject;
  const definition = createApp({
    state: {
      get: async key => saved.get(key) ?? null,
      transact: async ({ schemaVersion, checks, writes }) => {
        assert.equal(schemaVersion, 0);
        for (const check of checks) {
          if ((saved.get(check.key)?.version ?? null) !== check.version) throw new AppError('CONFLICT', 'Changed');
        }
        for (const write of writes) saved.set(write.val[0], { value: write.val[1], version: (saved.get(write.val[0])?.version ?? 0n) + 1n });
      },
    },
    events: {
      occurrenceId: (key, time) => `${key}@${time}`,
      emit: async occurrence => {
        calls.push(occurrence);
        if (reject === occurrence.automationId) throw new AppError('NOT_FOUND', 'Configure the automation');
        receipts.set(`${occurrence.automationId}:${occurrence.occurrenceId}`, occurrence);
      },
    },
  });
  for (const [name, handler] of Object.entries(definition.tools)) tools.set(name, input => handler(input));
  for (const [name, handler] of Object.entries(definition.events)) incoming.set(name, d => handler(d.payload, { occurrenceId: d.occurrenceId }));
  return { saved, receipts, incoming, tools, calls, reject: name => { reject = name; } };
}
function delivery(kind = 'pull_request_updated') {
  return {
    occurrenceId: 'source-occurrence-1',
    payload: {
      source: { generator_id: 'dev.chariox.github', connection_id: 'github-connection', event_type: kinds[kind], event_type_version: 1 },
      occurred_at: '2026-11-06T13:00:00.000Z',
      text: 'Review for charioxai/chariox #42 at head abcdef123456',
      metadata: { repository: { full_name: 'charioxai/chariox' }, pull_request: { number: 42, head: { sha: 'abcdef123456' } } },
      artifacts: [],
    },
  };
}
const configure = (k, automations) => k.tools.get('configure_automations')({ automations });

test('all five signed incoming and outgoing declarations match gateway types', () => {
  const events = JSON.parse(readFileSync(new URL('../bundle/schemas/events.json', import.meta.url))).events;
  const k = kernel();
  assert.deepEqual([...k.incoming.keys()].sort(), Object.keys(kinds).sort());
  for (const [kind, type] of Object.entries(kinds)) {
    const incoming = events.find(e => e.name === kind && e.direction === 'incoming');
    assert.deepEqual(incoming.payloadSchema.properties.source.properties.event_type.enum, [type]);
    assert.ok(events.some(e => e.name === `${kind}_notification` && e.direction === 'outgoing'));
  }
});

test('each event explicitly forwards to every configured automation with the original prompt', async () => {
  for (const kind of Object.keys(kinds)) {
    const k = kernel();
    await configure(k, { [`${kind}_notification`]: ['review-workflow', 'review-agent'] });
    await k.incoming.get(kind)(delivery(kind));
    assert.deepEqual(k.calls.map(c => c.automationId), ['review-workflow', 'review-agent']);
    for (const emitted of k.calls) {
      assert.deepEqual(JSON.parse(emitted.payload), delivery(kind).payload);
      assert.deepEqual(emitted.invocation, { prompt: delivery(kind).payload.text, artifacts: [] });
      assert.equal(emitted.occurredAtMs, BigInt(Date.parse(delivery(kind).payload.occurred_at)));
    }
  }
});

test('fan-out is not capped at the kernel transaction batch size', async () => {
  const k = kernel();
  const names = Array.from({ length: 20 }, (_, i) => `review-${i}`);
  await configure(k, { pull_request_updated_notification: names });
  await k.incoming.get('pull_request_updated')(delivery());
  assert.deepEqual(k.calls.map(c => c.automationId), names);
});

test('a refused target does not stop the others; retry after restart reuses every identity', async () => {
  const k = kernel();
  await configure(k, { pull_request_updated_notification: ['first', 'second', 'third'] });
  k.reject('first');
  await assert.rejects(k.incoming.get('pull_request_updated')(delivery()), { code: 'NOT_FOUND' });
  assert.deepEqual(k.calls.map(c => c.automationId), ['first', 'second', 'third']);
  assert.equal(k.receipts.size, 2);
  const restarted = kernel(k.saved, k.receipts);
  await restarted.incoming.get('pull_request_updated')(delivery());
  assert.equal(k.receipts.size, 3);
  assert.equal(k.calls[1].occurrenceId, restarted.calls[1].occurrenceId);
  assert.deepEqual(await restarted.tools.get('get_automations')({}), { automations: { pull_request_updated_notification: ['first', 'second', 'third'] } });
});

test('missing settings fail clearly; an explicit empty list disables forwarding', async () => {
  const k = kernel();
  await assert.rejects(k.incoming.get('pull_request_updated')(delivery()), { code: 'NOT_CONFIGURED' });
  await configure(k, { pull_request_updated_notification: [] });
  await k.incoming.get('pull_request_updated')(delivery());
  assert.equal(k.calls.length, 0);
});

test('settings reject duplicate or invalid names and unknown outgoing events', async () => {
  const k = kernel();
  for (const automations of [null, [], { unknown: ['a'] }, { pull_request_updated_notification: ['a', 'a'] }, { pull_request_updated_notification: [''] }, { pull_request_updated_notification: ['a b'] }]) {
    await assert.rejects(configure(k, automations), { code: 'INVALID_ARGUMENT' });
  }
});

test('a mismatched source or invalid occurrence time cannot be forwarded', async () => {
  const k = kernel();
  await configure(k, { pull_request_updated_notification: ['review'] });
  for (const mutate of [d => { d.payload.source.generator_id = 'other'; }, d => { d.payload.source.event_type = 'issues.opened'; }, d => { d.payload.occurred_at = 'invalid'; }]) {
    const d = delivery(); mutate(d);
    await assert.rejects(k.incoming.get('pull_request_updated')(d), { code: 'INVALID_ARGUMENT' });
  }
  assert.equal(k.calls.length, 0);
});
