// MP-08 / MP-09 / MP-10 / MP-11 AV3-AV6 slice 1 — the page-to-backend call port, without a browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { MAX_INPUT_BYTES, MAX_METHOD_LENGTH, MAX_PENDING_CALLS, readAnswer, readCall, relayCalls, serveCalls } from '../call-bridge.mjs';

const call = (id, method = 'add_note', input = { text: 'a' }) => ({ type: 'chariox.app-view.call', id, method, input });
const next = port => new Promise(resolve => { port.onmessage = event => resolve(event.data); });
const close = (...ports) => { for (const port of ports) port.close(); };

test('only a well-formed call is read; the method and input are bounded', () => {
  assert.deepEqual(readCall(call(1)), { id: 1, method: 'add_note', input: { text: 'a' } });
  assert.deepEqual(readCall(call(0, 'list_notes', {})), { id: 0, method: 'list_notes', input: {} });
  for (const bad of [null, 'x', [], call(-1), call(1.5), call(2 ** 53), call('1'), call(1, ''), call(1, 7), call(1, 'm'.repeat(MAX_METHOD_LENGTH + 1)),
    { ...call(1), extra: true }, { ...call(1), type: 'other' }, { ...call(1), input: undefined }, call(1, 'm', () => {}), call(1, 'm', 1n),
    call(1, 'm', 'x'.repeat(MAX_INPUT_BYTES + 1))]) assert.equal(readCall(bad), null, String(bad?.input ?? bad).slice(0, 60));
  const cycle = {}; cycle.self = cycle;
  assert.equal(readCall(call(1, 'm', cycle)), null);
});

test('only a well-formed answer is read', () => {
  assert.deepEqual(readAnswer({ type: 'chariox.app-view.result', id: 3, ok: true, result: { a: 1 } }), { id: 3, ok: true, result: { a: 1 } });
  assert.deepEqual(readAnswer({ type: 'chariox.app-view.result', id: 3, ok: false, error: { code: 'INVALID_INPUT', message: 'bad' } }), { id: 3, ok: false, error: { code: 'INVALID_INPUT', message: 'bad' } });
  for (const bad of [null, { type: 'chariox.app-view.result', id: 3 }, { type: 'chariox.app-view.result', id: 3, ok: false }, { type: 'chariox.app-view.result', id: 3, ok: false, error: { code: 1, message: 'x' } },
    { type: 'chariox.app-view.result', id: -3, ok: true, result: null }]) assert.equal(readAnswer(bad), null);
});

test('a served call answers the typed result, and a failure as a coded error without internals', async () => {
  const { port1: client, port2: peer } = new MessageChannel();
  const stop = serveCalls(client, async (method, input) => {
    if (method === 'ok') return { echoed: input };
    if (method === 'coded') throw Object.assign(new Error('Input does not match the declared schema'), { code: 'INVALID_INPUT' });
    throw new Error('/home/owner/secret.db: permission denied');
  });
  peer.postMessage(call(1, 'ok', { v: 1 }));
  assert.deepEqual(await next(peer), { type: 'chariox.app-view.result', id: 1, ok: true, result: { echoed: { v: 1 } } });
  peer.postMessage(call(2, 'coded', {}));
  assert.deepEqual(await next(peer), { type: 'chariox.app-view.result', id: 2, ok: false, error: { code: 'INVALID_INPUT', message: 'Input does not match the declared schema' } });
  peer.postMessage(call(3, 'boom', {}));
  assert.deepEqual(await next(peer), { type: 'chariox.app-view.result', id: 3, ok: false, error: { code: 'CALL_FAILED', message: 'The call did not complete' } });
  stop(); close(client, peer);
});

test('a malformed message on the served port is ignored and a flood is refused beyond the pending bound', async () => {
  const { port1: client, port2: peer } = new MessageChannel();
  const release = [];
  const stop = serveCalls(client, () => new Promise(resolve => release.push(resolve)));
  peer.postMessage({ nonsense: true });
  const answers = [];
  peer.onmessage = event => answers.push(event.data);
  for (let id = 0; id < MAX_PENDING_CALLS + 2; id++) peer.postMessage(call(id));
  while (answers.length < 2) await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(answers.map(row => [row.ok, row.error?.code]), [[false, 'TOO_MANY_CALLS'], [false, 'TOO_MANY_CALLS']]);
  assert.equal(release.length, MAX_PENDING_CALLS);
  stop(); close(client, peer);
});

test('the relay gives each call its own number toward the client and returns the answer under the page number', async () => {
  const app = new MessageChannel(), client = new MessageChannel();
  const stop = relayCalls(app.port1, client.port1);
  app.port2.postMessage(call(7, 'list_notes', {}));
  app.port2.postMessage(call(7, 'list_notes', { again: true }));
  const seen = [];
  client.port2.onmessage = event => seen.push(event.data);
  while (seen.length < 2) await new Promise(resolve => setTimeout(resolve, 5));
  assert.notEqual(seen[0].id, seen[1].id, 'a page cannot collide its own call numbers');
  const answers = [];
  app.port2.onmessage = event => answers.push(event.data);
  client.port2.postMessage({ type: 'chariox.app-view.result', id: seen[1].id, ok: true, result: 'second' });
  client.port2.postMessage({ type: 'chariox.app-view.result', id: seen[0].id, ok: true, result: 'first' });
  client.port2.postMessage({ type: 'chariox.app-view.result', id: 9999, ok: true, result: 'unknown' });
  while (answers.length < 2) await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(answers, [{ type: 'chariox.app-view.result', id: 7, ok: true, result: 'second' }, { type: 'chariox.app-view.result', id: 7, ok: true, result: 'first' }]);
  stop(); close(app.port1, app.port2, client.port1, client.port2);
});

test('the relay refuses a malformed page call by number and ends pending calls when stopped', async () => {
  const app = new MessageChannel(), client = new MessageChannel();
  const stop = relayCalls(app.port1, client.port1);
  const answers = [];
  app.port2.onmessage = event => answers.push(event.data);
  app.port2.postMessage(call(1, 'x'.repeat(MAX_METHOD_LENGTH + 1)));
  app.port2.postMessage(call(2));
  while (answers.length < 1) await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(answers[0], { type: 'chariox.app-view.result', id: 1, ok: false, error: { code: 'INVALID_CALL', message: 'The call is not a method name and a JSON input' } });
  stop();
  while (answers.length < 2) await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(answers[1], { type: 'chariox.app-view.result', id: 2, ok: false, error: { code: 'VIEW_CLOSED', message: 'The App view closed' } });
  close(app.port1, app.port2, client.port1, client.port2);
});

test('a call without an integer number cannot be answered, so it is dropped', async () => {
  const app = new MessageChannel(), client = new MessageChannel();
  const stop = relayCalls(app.port1, client.port1);
  const answers = [];
  app.port2.onmessage = event => answers.push(event.data);
  app.port2.postMessage({ type: 'chariox.app-view.call', method: 'm', input: {} });
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.deepEqual(answers, []);
  stop(); close(app.port1, app.port2, client.port1, client.port2);
});
