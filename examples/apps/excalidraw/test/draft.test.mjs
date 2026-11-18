// The window's draft client against a recording backend, without a browser: whatever order a window
// edits, saves, flushes and recovers in, its calls name only its own draft, and recovering reads the
// drafts of other windows without ever removing one.
import test from 'node:test';
import assert from 'node:assert/strict';
import { drafts } from '../src/draft.js';

const OPERATIONS = ['edit', 'saved', 'flush', 'recall'];

async function run(order) {
  const calls = [];
  let unsaved = null;
  const draft = drafts(async (name, input) => {
    calls.push({ name, input });
    if (name === 'list_drafts') return { drafts: [{ id: 'other', revision: 7 }] };
    if (name === 'read_draft') return { id: 'other', revision: 7, scene: { elements: [] } };
    return {};
  }, 'me', () => unsaved);
  for (const operation of order) {
    if (operation === 'edit') { unsaved = { elements: [{ id: 'e' }] }; draft.keep(); }
    if (operation === 'saved') { unsaved = null; await draft.drop(); }
    if (operation === 'flush') await draft.flush();
    if (operation === 'recall') await draft.recall();
  }
  await draft.drop();
  return calls;
}

function* orders(length, prefix = []) {
  if (prefix.length === length) { yield prefix; return; }
  for (const operation of OPERATIONS) yield* orders(length, [...prefix, operation]);
}

test('a window writes and removes only the draft of its own id, in every order of edits, saves, flushes and recoveries', async () => {
  let checked = 0;
  for (let length = 1; length <= 6; length++) {
    for (const order of orders(length)) {
      for (const { name, input } of await run(order)) {
        if (name === 'list_drafts' || name === 'read_draft') continue;
        assert.equal(input.id, 'me', `${order.join(' ')}: ${name} named ${input.id}`);
      }
      checked++;
    }
  }
  assert(checked > 5000, `${checked} orders were run`);
});

test('a window has no way to remove a draft that is not its own', () => {
  const draft = drafts(async () => ({}), 'me', () => null);
  assert.deepEqual(Object.keys(draft).sort(), ['drop', 'flush', 'keep', 'recall']);
});
