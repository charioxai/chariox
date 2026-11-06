import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const names = ['pull_request_opened', 'pull_request_updated', 'review_requested', 'issue_opened', 'workflow_run_completed']
  .map(name => `${name}_notification`);

async function view(saved = {}, fail = false) {
  const fields = names.map(name => ({
    name, value: '', attributes: new Set(),
    setAttribute(attribute) { this.attributes.add(attribute); },
    removeAttribute(attribute) { this.attributes.delete(attribute); },
  }));
  const form = { querySelectorAll: () => fields };
  let handler;
  const ambient = [], scoped = [];
  const chariox = {
    call: async method => { ambient.push(method); return { automations: saved }; },
    form: (element, registered) => { assert.ok(element === form); handler = registered; },
  };
  const context = { window: { chariox }, document: { getElementById: () => form } };
  vm.runInNewContext(await readFile(new URL('../bundle/ui/app.js', import.meta.url), 'utf8'), context);
  await new Promise(resolve => setImmediate(resolve));
  const caller = {
    call: async (method, input) => {
      scoped.push([method, input]);
      if (fail) throw new Error('refused');
      return { automations: input.automations };
    },
  };
  return { fields, handler, ambient, scoped, caller };
}

test('the form shows the saved automation names', async () => {
  const { fields, ambient } = await view({ [names[1]]: ['review', 'triage'] });
  assert.deepEqual(ambient, ['get_automations']);
  assert.equal(fields[1].value, 'review, triage');
  assert.equal(fields[0].value, '');
});

test('saving sends every event with trimmed names through the form caller', async () => {
  const { fields, handler, ambient, scoped, caller } = await view();
  ambient.length = 0;
  fields[1].value = ' review ,, triage ';
  await handler({}, caller);
  assert.deepEqual(ambient, []);
  assert.equal(JSON.stringify(scoped), JSON.stringify([['configure_automations', { automations: {
    ...Object.fromEntries(names.map(name => [name, []])),
    [names[1]]: ['review', 'triage'],
  } }]]));
  assert.equal(fields[1].value, 'review, triage');
});

test('a refused save marks the fields without showing text', async () => {
  const { fields, handler, caller } = await view({}, true);
  await handler({}, caller);
  assert.ok(fields.every(field => field.attributes.has('aria-invalid')));
});
