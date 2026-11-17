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
  assert.equal(manifest.displayName, 'Slack');
  assert.deepEqual(manifest.requiredFeatures, ['wasm-component', 'connections', 'directed-events'], 'the packer requires exactly the features the content uses, in its order');
  for (const path of [manifest.tools, manifest.events]) assert.ok(existsSync(new URL(`../${path}`, import.meta.url)), path);
  assert.ok(existsSync(new URL('../view/app.js', import.meta.url)), 'the page the client view is built from');
  for (const old of ['bundle', 'publisher.json']) assert.equal(existsSync(new URL(`../${old}`, import.meta.url)), false, `${old} belongs to the old runtime`);
});

test('every handler is declared and every declared tool and event has a handler', () => {
  assert.deepEqual(read(manifest.tools).tools.map(tool => tool.name).sort(), Object.keys(app.tools).sort());
  const events = read(manifest.events).events;
  assert.deepEqual(events.filter(event => event.direction === 'incoming').map(event => event.name).sort(), Object.keys(app.events).sort());
  assert.deepEqual(events.filter(event => event.direction === 'outgoing').map(event => event.name), ['notification']);
});

test('the App keeps the Slack connection grant it declared, and only the actions it uses', () => {
  assert.deepEqual(manifest.capabilities.connections, [{ actions: ['notification.reply', 'slack.reaction.add'], generator: 'dev.chariox.slack' }]);
});

test('only reads are replayed after an instance is lost, in the sorted order the packer requires', () => {
  assert.deepEqual(manifest.idempotentTools, ['list_notifications', 'view_changes']);
});
