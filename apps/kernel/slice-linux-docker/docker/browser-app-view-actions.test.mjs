import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import { createViewActions } from './browser-app-view-actions.mjs';

// Apps v2 A1: the page-side registry that A2 drives. Real-Chromium coverage is
// browser-app-view-actions.browser-test.mjs; this runs the logic with fake nodes.
function control(attrs, ownerForm) {
  const events = [];
  return { disabled: false, required: false, multiple: false, value: '', checked: false, ...attrs, events,
    closest: selector => (attrs.protected ? { selector } : null),
    dispatchEvent: event => events.push(event.type), checkValidity() { return !(this.required && this.value === ''); }, form: ownerForm,
    // A native click toggles a checkbox and selects a radio, then fires its events.
    click() { this.checked = this.type === 'radio' ? true : !this.checked; events.push('click'); } };
}
function form(name, attributes, controls) {
  const node = { elements: [], submitted: 0, valid: true,
    getAttribute: key => attributes[key] ?? null,
    checkValidity() { return this.elements.every(c => c.checkValidity()); },
    requestSubmit() { this.submitted++; } };
  node.elements = controls.map(c => control(c, node));
  return node;
}
// The prototype setters a framework's value tracker does not wrap; a write through
// them is recorded in `native`, a plain assignment is not.
const nativeSetter = () => ({ prototype: Object.defineProperty({}, 'value', {
  set(value) { this.native = [...(this.native ?? []), value]; this.value = value; } }) });
function page(forms = []) {
  const context = { document: { querySelectorAll: () => forms }, TextEncoder, Event: class { constructor(type) { this.type = type; } },
    HTMLInputElement: nativeSetter(), HTMLSelectElement: nativeSetter(), HTMLTextAreaElement: nativeSetter(),
    setTimeout, clearTimeout };
  const api = vm.runInNewContext(`(${createViewActions.toString()})()`, context);
  return { api, forms };
}
const closed = { type: 'object', additionalProperties: false };
const addTodo = { name: 'add_todo', description: 'Add a todo', inputSchema: { ...closed, properties: { text: { type: 'string' } } }, effect: 'write' };

test('a registered action is listed with the declaration shape and runs the user function', async () => {
  const { api } = page();
  const calls = [];
  const wrapped = api.action(addTodo, async params => { calls.push(params); return { id: 't1' }; });
  assert.deepEqual(JSON.parse(JSON.stringify(api.agent.registrations())), [addTodo]);
  assert.deepEqual(await wrapped({ text: 'milk' }), { id: 't1' });
  const answer = await api.agent.invoke('add_todo', { text: 'eggs' });
  assert.deepEqual(JSON.parse(JSON.stringify(answer)), { ok: true, value: { id: 't1' } });
  assert.deepEqual(calls.map(c => c.text), ['milk', 'eggs']);
});

test('registrations are copies and a bad declaration fails at registration', () => {
  const { api } = page();
  api.action(addTodo, async () => null);
  api.agent.registrations()[0].effect = 'approval';
  assert.equal(api.agent.registrations()[0].effect, 'write');
  const handler = async () => null;
  for (const bad of [
    { ...addTodo, name: 'Bad-Name' },
    { ...addTodo, name: 'other', effect: 'delete' },
    { ...addTodo, name: 'other', effect: 'write', approvalReason: 'why' },
    { ...addTodo, name: 'other', effect: 'approval' },
    { ...addTodo, name: 'other', extra: true },
  ]) assert.throws(() => api.action(bad, handler), { name: 'TypeError' });
  assert.throws(() => api.action(addTodo, handler), /already registered/);
  assert.throws(() => api.action({ ...addTodo, name: 'second' }, 'not a function'), { name: 'TypeError' });
});

test('action failures keep the app error code; unknown and duplicate names are named', async () => {
  const { api } = page([form('f', { 'data-chariox-action': 'add_todo' }, [])]);
  api.action({ ...addTodo, name: 'fails' }, async () => { throw Object.assign(new Error('no room'), { code: 'APP_STORAGE_FULL' }); });
  assert.deepEqual(JSON.parse(JSON.stringify(await api.agent.invoke('fails', {}))), { ok: false, code: 'APP_STORAGE_FULL', message: 'no room' });
  assert.equal((await api.agent.invoke('nothing', {})).code, 'unknown_action');
  api.action(addTodo, async () => null);
  assert.equal((await api.agent.invoke('add_todo', {})).code, 'duplicate_action');
});

test('results and state are bounded JSON copies', async () => {
  const { api } = page();
  api.action({ ...addTodo, name: 'big' }, async () => 'x'.repeat(300 * 1024));
  api.action({ ...addTodo, name: 'cyclic' }, async () => { const o = {}; o.self = o; return o; });
  assert.equal((await api.agent.invoke('big', {})).code, 'result_too_large');
  assert.equal((await api.agent.invoke('cyclic', {})).code, 'not_json');
  assert.equal((await api.agent.state()).code, 'state_unavailable');
  let size = 10;
  api.view.state(() => ({ text: 'x'.repeat(size) }));
  assert.deepEqual(JSON.parse(JSON.stringify(await api.agent.state())), { ok: true, value: { text: 'xxxxxxxxxx' } });
  size = 70 * 1024;
  assert.equal((await api.agent.state()).code, 'state_too_large');
  assert.throws(() => api.view.state('nope'), { name: 'TypeError' });
});

test('changes are sequenced, bounded and can be waited on', async () => {
  const { api } = page();
  const read = async (...args) => JSON.parse(JSON.stringify((await api.agent.changes(...args)).value));
  assert.deepEqual(await read(0), { latest: 0, dropped: false, changes: [] });
  const waiting = read(0, 5000);
  api.view.changed({ todos: 1 });
  assert.deepEqual(await waiting, { latest: 1, dropped: false, changes: [{ seq: 1, delta: { todos: 1 } }] });
  const started = Date.now();
  assert.deepEqual(await read(1, 20), { latest: 1, dropped: false, changes: [] });
  assert(Date.now() - started >= 15);
  for (let i = 2; i <= 300; i++) api.view.changed({ i });
  const late = await read(1);
  assert.equal(late.dropped, true, 'the reader missed evicted changes and must re-read state');
  assert.equal(late.changes.length, 256);
  assert.equal((await read(250)).dropped, false);
  assert.throws(() => api.view.changed('x'.repeat(20 * 1024)), { name: 'RangeError' });
});

test('a form is an action whose schema comes from its simple controls', async () => {
  const rename = form('rename', { 'data-chariox-action': 'rename_list', 'data-chariox-effect': 'write', 'data-chariox-description': 'Rename the list' }, [
    { tagName: 'INPUT', type: 'text', name: 'title', required: true },
    { tagName: 'INPUT', type: 'number', name: 'position' },
    { tagName: 'INPUT', type: 'checkbox', name: 'pinned' },
    { tagName: 'INPUT', type: 'radio', name: 'color', value: 'red' },
    { tagName: 'INPUT', type: 'radio', name: 'color', value: 'blue' },
    { tagName: 'SELECT', name: 'sort', options: [{ value: 'a' }, { value: 'b' }, { value: 'c', disabled: true }] },
    { tagName: 'TEXTAREA', name: 'note' },
    { tagName: 'INPUT', type: 'password', name: 'pw' },
    { tagName: 'INPUT', type: 'hidden', name: 'csrf' },
    { tagName: 'INPUT', type: 'text', name: 'card', protected: true },
    { tagName: 'INPUT', type: 'text', name: 'off', disabled: true },
    { tagName: 'INPUT', type: 'submit', name: 'go' },
  ]);
  const { api } = page([rename]);
  assert.deepEqual(JSON.parse(JSON.stringify(api.agent.registrations())), [{
    name: 'rename_list', description: 'Rename the list', effect: 'write',
    inputSchema: { type: 'object', additionalProperties: false, required: ['title'], properties: {
      title: { type: 'string' }, position: { type: 'number' }, pinned: { type: 'boolean' },
      color: { type: 'string', enum: ['red', 'blue'] }, sort: { type: 'string', enum: ['a', 'b'] }, note: { type: 'string' },
      pw: { type: 'string' }, card: { type: 'string' } } },
  }]);
  const answer = await api.agent.invoke('rename_list', { title: 'Groceries', pinned: true, color: 'blue', sort: 'b', pw: 'hunter2', card: '4111' });
  assert.deepEqual(JSON.parse(JSON.stringify(answer)), { ok: true, value: { submitted: true } });
  const byName = Object.fromEntries(rename.elements.map(c => [c.name + (c.type === 'radio' ? c.value : ''), c]));
  assert.equal(byName.title.value, 'Groceries');
  assert.equal(byName.pinned.checked, true);
  assert.equal(byName.colorblue.checked, true);
  assert.equal(byName.colorred.checked, false);
  assert.equal(byName.sort.value, 'b');
  // Values the agent knows go into password and protected controls like any other.
  assert.equal(byName.pw.value, 'hunter2');
  assert.equal(byName.card.value, '4111');
  assert.deepEqual(byName.title.events, ['input', 'change']);
  assert.deepEqual(byName.title.native, ['Groceries'], 'text goes through the prototype setter, which a framework value tracker does not intercept');
  assert.deepEqual(byName.sort.native, ['b']);
  // A checkbox and a radio change by a native click, the event a framework listens to.
  assert.deepEqual(byName.pinned.events, ['click']);
  assert.deepEqual(byName.colorblue.events, ['click']);
  assert.deepEqual(byName.colorred.events, []);
  assert.equal(rename.submitted, 1);
});

test('a form refuses what it cannot take and approval markup carries its reason', async () => {
  const strict = form('strict', { 'data-chariox-action': 'wipe', 'data-chariox-effect': 'approval', 'data-chariox-approval-reason': 'Deletes all' }, [
    { tagName: 'INPUT', type: 'text', name: 'title', required: true },
    { tagName: 'INPUT', type: 'password', name: 'pw' },
  ]);
  const { api } = page([strict]);
  assert.equal(api.agent.registrations()[0].approvalReason, 'Deletes all');
  for (const params of [{ nope: 'x' }, { title: 5 }, { pw: 5 }]) assert.equal((await api.agent.invoke('wipe', params)).code, 'invalid_params');
  assert.equal((await api.agent.invoke('wipe', {})).code, 'invalid_form');
  assert.equal(strict.submitted, 0);
  assert.equal((await api.agent.invoke('wipe', { title: 'x', pw: 'secret' })).ok, true, 'an agent may enter a password it knows');
  assert.equal(strict.elements[1].value, 'secret');
  assert.equal(strict.submitted, 1);
});
