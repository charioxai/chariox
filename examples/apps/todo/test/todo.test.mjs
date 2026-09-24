// Exercises the Todo backend against an in-memory stand-in for the kernel's
// App state (compare-and-set), wake and occurrence contracts. The kernel's own
// implementations of those contracts are tested in the kernel crates.
import assert from 'node:assert/strict';
import test from 'node:test';
import { occurrenceId } from '../../../../packages/app-sdk/src/occurrences.js';
import register from '../bundle/runtime/main.mjs';

function fakeKernel() {
  const state = new Map();
  const wakes = new Map();
  const occurrences = [];
  const tools = new Map();
  let onWake;
  const chariox = {
    tools: { register: (name, handler) => tools.set(name, handler) },
    events: { occurrenceId },
    schedule: { onWake: handler => { onWake = handler; } },
    state: {
      async get(key) { return state.get(key) ?? null; },
      async transaction({ checks, writes, wakes: changes = [], occurrences: emitted = [] }) {
        for (const check of checks) {
          if ((state.get(check.key)?.version ?? null) !== check.version) {
            throw Object.assign(new Error('conflict'), { code: 'CONFLICT' });
          }
        }
        for (const write of writes) state.set(write.key, { value: write.value, version: (state.get(write.key)?.version ?? 0) + 1 });
        for (const change of changes) {
          if (change.op === 'cancel') wakes.delete(change.id);
          else wakes.set(change.id, { id: change.id, dueAtMs: change.dueAtMs, revision: change.revision });
        }
        occurrences.push(...emitted);
        return { revision: 1, receipts: [] };
      },
    },
  };
  register(chariox);
  // Like the kernel, a successful delivery completes the wake at its revision.
  async function wake(delivered) {
    await onWake({ ...delivered, overdue: false });
    const current = wakes.get(delivered.id);
    if (current?.revision === delivered.revision && current.dueAtMs === delivered.dueAtMs) wakes.delete(delivered.id);
  }
  return { tools, wakes, occurrences, wake };
}

test('Todos are created, listed, completed and deleted with their wakes kept in step', async () => {
  const kernel = fakeKernel();
  const todo = await kernel.tools.get('create_todo')({ title: 'Ship App views', due_at_ms: 5000 });
  assert.equal(todo.title, 'Ship App views');
  assert.deepEqual(kernel.wakes.get(`todo-${todo.id}`), { id: `todo-${todo.id}`, dueAtMs: 5000, revision: '1' });
  await kernel.tools.get('create_todo')({ title: 'No due time' });
  assert.equal((await kernel.tools.get('list_todos')({})).todos.length, 2);
  await kernel.tools.get('complete_todo')({ id: todo.id, done: true });
  assert.equal(kernel.wakes.has(`todo-${todo.id}`), false, 'completing a Todo cancels its reminder');
  assert.equal((await kernel.tools.get('list_todos')({ open_only: true })).todos.length, 1);
  await kernel.tools.get('delete_todo')({ id: todo.id });
  assert.equal((await kernel.tools.get('list_todos')({})).todos.length, 1);
  await assert.rejects(kernel.tools.get('delete_todo')({ id: todo.id }), /No Todo/);
});

test('a due wake emits one todo_due occurrence and ignores stale revisions', async () => {
  const kernel = fakeKernel();
  const todo = await kernel.tools.get('create_todo')({ title: 'Call the bank', due_at_ms: 1000 });
  // An edit moves the due time; the earlier wake revision is stale.
  await kernel.tools.get('update_todo')({ id: todo.id, due_at_ms: 2000 });
  await kernel.wake({ id: `todo-${todo.id}`, dueAtMs: 1000, revision: '1' });
  assert.equal(kernel.occurrences.length, 0);
  const current = kernel.wakes.get(`todo-${todo.id}`);
  await kernel.wake(current);
  await kernel.wake(current);
  assert.equal(kernel.occurrences.length, 1, 'a redelivered wake is idempotent');
  const [occurrence] = kernel.occurrences;
  assert.equal(occurrence.automationId, 'reminders');
  assert.equal(occurrence.occurredAtMs, 2000);
  assert.deepEqual(occurrence.payload, { todo_id: todo.id, title: 'Call the bank', due_at_ms: 2000 });
  assert.match(occurrence.invocation.prompt, /Call the bank/);
  assert.equal(occurrence.occurrenceId, occurrenceId(`todo:${todo.id}:2`, 2000));
  assert.equal(kernel.wakes.has(`todo-${todo.id}`), false, 'a reminded Todo holds no wake');
});
