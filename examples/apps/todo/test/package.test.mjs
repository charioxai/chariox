// The signed declarations and the handlers say the same thing, and nothing of the old Node runtime is left.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { createApp } from '../handler.mjs';

const read = path => JSON.parse(readFileSync(new URL(`../${path}`, import.meta.url), 'utf8'));
const app = createApp({});
const manifest = read('app.json');

test('the manifest is a v2 package that names files which exist', () => {
  assert.equal(manifest.schema, 'chariox.app.v2');
  assert.equal(manifest.runtime.engine, 'wasm');
  assert.equal(manifest.displayName, 'Todo');
  assert.deepEqual(manifest.requiredFeatures, ['wasm-component', 'directed-events'], 'the packer requires exactly the features the content uses, in its order');
  for (const path of [manifest.icon, manifest.tools, manifest.events]) assert.ok(existsSync(new URL(`../${path}`, import.meta.url)), path);
  for (const old of ['bundle', 'publisher.json']) assert.equal(existsSync(new URL(`../${old}`, import.meta.url)), false, `${old} belongs to the old runtime`);
});

test('every handler is declared and every declared tool and event has a handler', () => {
  assert.deepEqual(read(manifest.tools).tools.map(tool => tool.name).sort(), Object.keys(app.tools).sort());
  const events = read(manifest.events).events;
  assert.deepEqual(events.filter(event => event.direction === 'incoming').map(event => event.name), Object.keys(app.events));
  assert.deepEqual(events.filter(event => event.direction === 'outgoing').map(event => event.name), ['todo_due']);
});

test('only reads are replayed after an instance is lost, in the sorted order the packer requires', () => {
  assert.deepEqual(manifest.idempotentTools, ['list_todos', 'reminder_status', 'view_changes']);
});

test('a request cannot carry notes the change feed could not hold', () => {
  const requested = read(manifest.events).events.find(event => event.name === 'todo_requested');
  const tool = read(manifest.tools).tools.find(item => item.name === 'create_todo');
  assert.equal(requested.payloadSchema.properties.notes.maxLength, tool.inputSchema.properties.notes.maxLength);
});
