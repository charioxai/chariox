import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

test('Todo form keeps its caller for the save and awaited refresh', async () => {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, { value: id === 'title' ? 'Draft' : '', children: [],
      classList: { toggle() {}, contains: () => false }, addEventListener() {}, replaceChildren() {}, reset() {}, focus() {} });
    return elements.get(id);
  };
  let handler;
  const ambient = [], scoped = [];
  const answer = method => method === 'list_todos' ? { todos: [] } : method === 'reminder_status' ? { state: 'missing' } : {};
  const chariox = {
    call: async method => { ambient.push(method); return answer(method); },
    form: (_element, registered) => { handler = registered; },
  };
  const context = { window: { chariox }, document: { getElementById: element }, setInterval() {}, Date };
  vm.runInNewContext(await readFile(new URL('../bundle/ui/app.js', import.meta.url), 'utf8'), context);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(typeof handler, 'function', 'the async form explicitly registers its handler');
  ambient.length = 0;
  await handler({}, { call: async method => { scoped.push(method); return answer(method); } });
  assert.deepEqual(scoped, ['create_todo', 'list_todos']);
  assert.deepEqual(ambient, []);
});
