import assert from 'node:assert/strict';
import test from 'node:test';
import { applyChange, awaitsReminder, follow } from '../view/model.js';

const todo = (id, extra = {}) => ({ id, title: `Todo ${id}`, done: false, reminded: false, due_at_ms: null, ...extra });

test('a change puts a Todo in place, adds a new one at the end and deletes by id', () => {
  const list = [todo('a'), todo('b')];
  assert.deepEqual(applyChange(list, { todo: todo('b', { done: true }) }), [todo('a'), todo('b', { done: true })]);
  assert.deepEqual(applyChange(list, { todo: todo('c') }).map(item => item.id), ['a', 'b', 'c']);
  assert.deepEqual(applyChange(list, { deleted: 'a' }), [todo('b')]);
  assert.deepEqual(applyChange(list, { deleted: 'missing' }), list);
});

test('replaying changes the window already holds ends in the same list', () => {
  const changes = [{ todo: todo('a') }, { todo: todo('a', { done: true }) }, { todo: todo('b') }, { deleted: 'a' }];
  const once = changes.reduce(applyChange, []);
  const replayed = [...changes, ...changes].reduce(applyChange, []);
  assert.deepEqual(once.map(item => item.id), ['b']);
  assert.deepEqual(replayed.map(item => item.id), ['b']);
});

test('only a Todo with a due time that is open and not yet reminded waits for its reminder', () => {
  assert.equal(awaitsReminder(todo('a', { due_at_ms: 5 })), true);
  assert.equal(awaitsReminder(todo('a')), false);
  assert.equal(awaitsReminder(todo('a', { due_at_ms: 5, done: true })), false);
  assert.equal(awaitsReminder(todo('a', { due_at_ms: 5, reminded: true })), false);
});

test('a round of the feed applies the changes and moves the position; a window that fell behind reloads', async () => {
  const applied = [], waits = [];
  let reloads = 0;
  const sinks = { apply: delta => applied.push(delta), reload: async () => { reloads++; } };
  const changes = answer => async (since, waitMs) => { waits.push([since, waitMs]); return answer; };
  assert.equal(await follow(changes({ latest: 7, dropped: false, changes: [{ seq: 6, delta: { deleted: 'a' } }, { seq: 7, delta: { todo: todo('b') } }] }), 5, sinks), 7);
  assert.deepEqual(applied, [{ deleted: 'a' }, { todo: todo('b') }]);
  assert.equal(reloads, 0);
  assert.equal(await follow(changes({ latest: 300, dropped: true, changes: [{ seq: 173, delta: { deleted: 'x' } }] }), 7, sinks), 300);
  assert.equal(reloads, 1);
  assert.equal(applied.length, 2, 'a dropped answer is not applied piecemeal');
  assert.deepEqual(waits, [[5, 20000], [7, 20000]]);
});
