// Exercises the Todo handlers against an in-memory stand-in for the kernel's App state (compare-and-set,
// wakes, occurrences, as the wasm imports see them: JSON text values, bigint versions and times, tagged
// variants). The kernel's own implementations of those contracts are tested in the kernel crates.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createFeed } from '../../../../packages/app-sdk-js/src/feed.js';
import { createApp } from '../handler.mjs';

const refusal = (code, message = code) => ({ code, message, retryable: false });

function fakeKernel({ automation = true, automations = [], scheduled = false } = {}) {
  const records = new Map();
  const wakes = new Map();
  const occurrences = [];
  const state = {
    async get(key) { return records.get(key) ?? null; },
    async transact({ schemaVersion, checks, writes, wakes: changes = [], occurrences: emitted = [] }) {
      // Like the kernel: the package declares no migrations, so its data schema is 0.
      if (schemaVersion !== 0) throw refusal('SCHEMA_MISMATCH');
      // Like the kernel, an occurrence for an unconfigured automation rolls back the whole transaction.
      if (emitted.length && !automation) throw { payload: refusal('NOT_FOUND') };
      if (emitted.some(event => (event.scheduleRevision !== null) !== scheduled)) throw { payload: refusal('INVALID_ARGUMENT') };
      for (const check of checks) {
        if ((records.get(check.key)?.version ?? null) !== check.version) throw { payload: refusal('CONFLICT') };
      }
      for (const write of writes) {
        if (write.tag === 'put') records.set(write.val[0], { value: write.val[1], version: (records.get(write.val[0])?.version ?? 0n) + 1n });
        else records.delete(write.val);
      }
      for (const change of changes) {
        if (change.tag === 'cancel') wakes.delete(change.val);
        else wakes.set(change.val.id, change.val);
      }
      occurrences.push(...emitted);
      return { revision: 1n, receipts: [] };
    },
  };
  const events = {
    occurrenceId: (key, time) => `evt1.${time}.${key}`,
    async automations() { return automations; },
  };
  const app = createApp({ state, events, feed: createFeed(state) });
  // Like the kernel, a successful delivery completes the wake at its revision.
  async function wake(delivered) {
    await app.wake(delivered, { overdue: false });
    const current = wakes.get(delivered.id);
    if (current?.revision === delivered.revision && current.dueAtMs === delivered.dueAtMs) wakes.delete(delivered.id);
  }
  return { tools: app.tools, incoming: app.events, wakes, occurrences, wake, records };
}
const person = { actor: { kind: 'human', id: 'owner' } };
const agent = { actor: { kind: 'agent', id: 'agent-7' } };
const wakeOf = (id, dueAtMs, revision) => ({ id: `todo-${id}`, dueAtMs, revision });

test('Todos are created, listed, completed and deleted with their wakes kept in step', async () => {
  const kernel = fakeKernel();
  const todo = await kernel.tools.create_todo({ title: 'Ship App views', due_at_ms: 5000 }, person);
  assert.equal(todo.title, 'Ship App views');
  assert.deepEqual(kernel.wakes.get(`todo-${todo.id}`), wakeOf(todo.id, 5000n, '1'));
  await kernel.tools.create_todo({ title: 'No due time' }, person);
  assert.equal((await kernel.tools.list_todos({})).todos.length, 2);
  await kernel.tools.complete_todo({ id: todo.id, done: true }, person);
  assert.equal(kernel.wakes.has(`todo-${todo.id}`), false, 'completing a Todo cancels its reminder');
  assert.equal((await kernel.tools.list_todos({ open_only: true })).todos.length, 1);
  await kernel.tools.delete_todo({ id: todo.id }, person);
  assert.equal((await kernel.tools.list_todos({})).todos.length, 1);
  await assert.rejects(kernel.tools.delete_todo({ id: todo.id }, person), { code: 'NOT_FOUND', message: `No Todo with id ${todo.id}` });
});

test('every change reaches the feed in the transaction that made it', async () => {
  const kernel = fakeKernel();
  const todo = await kernel.tools.create_todo({ title: 'Feed me' }, person);
  await kernel.tools.complete_todo({ id: todo.id, done: true }, person);
  await kernel.tools.delete_todo({ id: todo.id }, person);
  await assert.rejects(kernel.tools.delete_todo({ id: todo.id }, person), { code: 'NOT_FOUND' });
  const { latest, changes, dropped } = await kernel.tools.view_changes({ since: 0 });
  assert.equal(latest, 3, 'a refused change is not in the feed');
  assert.equal(dropped, false);
  assert.deepEqual(changes[0].delta, { todo });
  assert.deepEqual(changes[1].delta, { todo: { ...todo, done: true, revision: 2 } });
  assert.deepEqual(changes[2].delta, { deleted: todo.id });
  assert.deepEqual((await kernel.tools.view_changes({ since: 2 })).changes, [changes[2]]);
});

test('a due wake emits one todo_due occurrence and ignores stale revisions', async () => {
  const kernel = fakeKernel();
  const todo = await kernel.tools.create_todo({ title: 'Call the bank', due_at_ms: 1000 }, person);
  // An edit moves the due time; the earlier wake revision is stale.
  await kernel.tools.update_todo({ id: todo.id, due_at_ms: 2000 }, person);
  await kernel.wake(wakeOf(todo.id, 1000n, '1'));
  assert.equal(kernel.occurrences.length, 0);
  const current = kernel.wakes.get(`todo-${todo.id}`);
  await kernel.wake(current);
  await kernel.wake(current);
  assert.equal(kernel.occurrences.length, 1, 'a redelivered wake is idempotent');
  const [occurrence] = kernel.occurrences;
  assert.equal(occurrence.automationId, 'reminders');
  assert.equal(occurrence.occurredAtMs, 2000n);
  assert.deepEqual(JSON.parse(occurrence.payload), { todo_id: todo.id, title: 'Call the bank', due_at_ms: 2000 });
  assert.match(occurrence.invocation.prompt, /Call the bank/);
  assert.deepEqual(occurrence.invocation.artifacts, []);
  assert.equal(occurrence.occurrenceId, `evt1.2000.todo:${todo.id}:2`);
  assert.equal(kernel.wakes.has(`todo-${todo.id}`), false, 'a reminded Todo holds no wake');
  const { changes } = await kernel.tools.view_changes({ since: 0 });
  assert.equal(changes.at(-1).delta.todo.reminded, true, 'a window shows the reminder as sent');
  assert.equal(changes.length, 3, 'the stale and the repeated wake changed nothing');
});

test('a reminder is recorded without an occurrence when no automation is configured', async () => {
  const kernel = fakeKernel({ automation: false });
  const todo = await kernel.tools.create_todo({ title: 'Water plants', due_at_ms: 1000 }, person);
  await kernel.wake(kernel.wakes.get(`todo-${todo.id}`));
  assert.equal(kernel.occurrences.length, 0);
  const [stored] = (await kernel.tools.list_todos({})).todos;
  assert.equal(stored.reminded, true, 'the wake does not keep failing and retrying');
  assert.equal(kernel.wakes.has(`todo-${todo.id}`), false);
});

test('App errors reach callers with a code, and limits match the kernel', async () => {
  const kernel = fakeKernel();
  await assert.rejects(kernel.tools.delete_todo({ id: 'missing' }, person), { code: 'NOT_FOUND' });
  for (let index = 0; index < 150; index += 1) await kernel.tools.create_todo({ title: `t${index}` }, person);
  await assert.rejects(kernel.tools.create_todo({ title: 'one more' }, person), { code: 'LIMIT_EXCEEDED' });
});

test('the App reports its own limit before the kernel value cap, for any text', async () => {
  const kernel = fakeKernel();
  const notes = '\u{1F600}'.repeat(1000);
  let created = 0;
  await assert.rejects(async () => {
    for (; created < 150; created += 1) await kernel.tools.create_todo({ title: `t${created}`, notes }, person);
  }, error => error.code === 'LIMIT_EXCEEDED' && /too large/.test(error.message));
  assert.ok(created > 0 && created < 150);
});

test('an incoming request creates one Todo even when it is delivered twice', async () => {
  const kernel = fakeKernel();
  const deliver = () => kernel.incoming.todo_requested({ title: 'From Slack', due_at_ms: 9000 }, { occurrenceId: 'occ-1' });
  const todo = await deliver();
  assert.equal(todo.title, 'From Slack');
  assert.equal(await deliver(), null);
  const { todos } = await kernel.tools.list_todos({});
  assert.equal(todos.length, 1);
  assert.deepEqual(kernel.wakes.get(`todo-${todo.id}`), wakeOf(todo.id, 9000n, '1'));
  assert.equal((await kernel.tools.view_changes({})).latest, 1, 'the repeated delivery wrote nothing');
});

test('reminder status reports the reminders automation without naming its workflow', async () => {
  const status = options => fakeKernel(options).tools.reminder_status({});
  const reminders = { automationId: 'reminders', event: 'todo_due', eventVersion: 1, state: 'broken',
    lastReceipt: { receiptId: 'receipt-1', state: 'failed' } };
  const other = { ...reminders, automationId: 'other', state: 'active', lastReceipt: null };
  assert.deepEqual(await status({ automations: [other, reminders] }), { state: 'broken', last_delivery: 'failed' });
  assert.deepEqual(await status({ automations: [other] }), { state: 'missing', last_delivery: null });
});

// MP-08/MP-10/MP-11: the App sees who called (human owner vs agent) and treats them differently.
test('an agent may delete only Todos an agent created; a person may delete any', async () => {
  const kernel = fakeKernel();
  const mine = await kernel.tools.create_todo({ title: 'Person Todo' }, person);
  const theirs = await kernel.tools.create_todo({ title: 'Agent Todo' }, agent);
  assert.equal(mine.created_by, 'person');
  assert.equal(theirs.created_by, 'agent');
  await assert.rejects(kernel.tools.delete_todo({ id: mine.id }, agent), { code: 'FORBIDDEN' });
  assert.deepEqual(await kernel.tools.delete_todo({ id: theirs.id }, agent), { deleted: theirs.id });
  assert.deepEqual(await kernel.tools.delete_todo({ id: mine.id }, person), { deleted: mine.id });
});

for (const scheduled of [false, true]) {
  test(`MP-08/MP-10 Todo reminders work with ${scheduled ? 'scheduled' : 'ordinary'} automations`, async () => {
    const kernel = fakeKernel({ scheduled });
    const todo = await kernel.tools.create_todo({ title: 'Review public repo', due_at_ms: 1000 }, person);
    const wake = kernel.wakes.get(`todo-${todo.id}`);
    await kernel.wake(wake);
    await kernel.wake(wake);
    assert.equal(kernel.occurrences.length, 1);
    const [event] = kernel.occurrences;
    assert.equal(event.scheduleRevision, scheduled ? '1' : null);
    assert.equal(event.occurrenceId, `evt1.1000.todo:${todo.id}:1`);
    assert.equal((await kernel.tools.list_todos({})).todos[0].reminded, true);
    assert.equal(kernel.wakes.size, 0);
  });
}
