// MP-08 / MP-09 / MP-10 / MP-11 AV3-AV6 slice 1 — the page SDK shim, run exactly as the App document serializes it.
import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { bootstrap } from '../bootstrap.mjs';

function page() {
  const listeners = [];
  const parent = {};
  const sandbox = { parent, MessageChannel, setTimeout, addEventListener: (type, listener) => listeners.push([type, listener]) };
  vm.createContext(sandbox);
  vm.runInContext('(' + bootstrap.toString() + ')();', sandbox);
  const deliver = (source, data, ports) => { for (const [, listener] of listeners) listener({ source, data, ports }); };
  return { sandbox, parent, deliver, chariox: sandbox.chariox };
}
const portMessage = { type: 'chariox.app-view.port' };
const next = port => new Promise(resolve => { port.onmessage = event => resolve(event.data); });

test('window.chariox is a frozen surface of the call and the change feed', () => {
  const { sandbox, chariox } = page();
  assert.deepEqual(Object.keys(chariox), ['call', 'changes']);
  assert.equal(Object.isFrozen(chariox), true);
  assert.equal(Object.getOwnPropertyDescriptor(sandbox, 'chariox').configurable, false);
  assert.equal(Object.getOwnPropertyDescriptor(sandbox, 'chariox').writable, false);
});

test('a call made before the host delivers the port waits, then travels and resolves with the typed result', async () => {
  const { parent, deliver, chariox } = page();
  const channel = new MessageChannel();
  const seen = [];
  channel.port1.onmessage = event => seen.push(event.data);
  const first = chariox.call('add_note', { text: 'a' });
  const second = chariox.call('list_notes');
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.deepEqual(seen, [], 'nothing is sent before the port arrives');
  deliver(parent, portMessage, [channel.port2]);
  while (seen.length < 2) await new Promise(resolve => setTimeout(resolve, 5));
  assert.deepEqual(seen, [{ type: 'chariox.app-view.call', id: 0, method: 'add_note', input: { text: 'a' } }, { type: 'chariox.app-view.call', id: 1, method: 'list_notes', input: {} }]);
  channel.port1.postMessage({ type: 'chariox.app-view.result', id: 1, ok: true, result: [1, 2] });
  channel.port1.postMessage({ type: 'chariox.app-view.result', id: 0, ok: false, error: { code: 'INVALID_INPUT', message: 'text is too long' } });
  assert.deepEqual(await second, [1, 2]);
  await assert.rejects(first, error => error.code === 'INVALID_INPUT' && error.message === 'text is too long');
  channel.port1.close(); channel.port2.close();
});

test('only the first port from the parent is taken; other windows and later ports are ignored', async () => {
  const { parent, deliver, chariox } = page();
  const stranger = new MessageChannel(), first = new MessageChannel(), later = new MessageChannel();
  deliver({}, portMessage, [stranger.port2]);
  deliver(parent, { type: 'other' }, [stranger.port2]);
  deliver(parent, portMessage, []);
  deliver(parent, portMessage, [first.port2]);
  deliver(parent, portMessage, [later.port2]);
  const answered = next(first.port1);
  const call = chariox.call('list_notes', {});
  assert.deepEqual(await answered, { type: 'chariox.app-view.call', id: 0, method: 'list_notes', input: {} });
  first.port1.postMessage({ type: 'chariox.app-view.result', id: 0, ok: true, result: null });
  assert.equal(await call, null);
  let leaked = false;
  later.port1.onmessage = stranger.port1.onmessage = () => { leaked = true; };
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.equal(leaked, false);
  for (const channel of [stranger, first, later]) { channel.port1.close(); channel.port2.close(); }
});

test('an input that cannot be sent rejects the call and leaves nothing waiting', async () => {
  const { parent, deliver, chariox } = page();
  const channel = new MessageChannel();
  deliver(parent, portMessage, [channel.port2]);
  await assert.rejects(chariox.call('add_note', { text: () => {} }), error => error.name === 'DataCloneError' || /clone/i.test(error.message));
  const answered = next(channel.port1);
  chariox.call('list_notes', {});
  assert.equal((await answered).id, 1, 'the next call is numbered after the refused one');
  channel.port1.close(); channel.port2.close();
});

test('an answer that is not for a waiting call is ignored', async () => {
  const { parent, deliver, chariox } = page();
  const channel = new MessageChannel();
  deliver(parent, portMessage, [channel.port2]);
  const sent = next(channel.port1);
  const call = chariox.call('list_notes', {});
  await sent;
  channel.port1.postMessage({ type: 'chariox.app-view.result', id: 77, ok: true, result: 'stray' });
  channel.port1.postMessage({ type: 'unrelated', id: 0 });
  channel.port1.postMessage({ type: 'chariox.app-view.result', id: 0, ok: true, result: 'mine' });
  assert.equal(await call, 'mine');
  channel.port1.close(); channel.port2.close();
});

// The change feed a page reads: `view_changes` through the same call, waiting up to `waitMs` for a change.
function feedPage(answers) {
  const { parent, deliver, chariox } = page();
  const channel = new MessageChannel();
  const seen = [];
  channel.port1.onmessage = event => {
    seen.push(event.data);
    const answer = answers.length > 1 ? answers.shift() : answers[0];
    channel.port1.postMessage({ type: 'chariox.app-view.result', id: event.data.id, ok: true, result: answer });
  };
  deliver(parent, portMessage, [channel.port2]);
  return { chariox, seen, close: () => { channel.port1.close(); channel.port2.close(); } };
}
const quiet = { latest: 4, dropped: false, changes: [] };

test('changes reads the feed after a position and returns at once when there is something to apply', async () => {
  const change = { latest: 5, dropped: false, changes: [{ seq: 5, delta: { x: 1 } }] };
  const { chariox, seen, close } = feedPage([change]);
  assert.deepEqual(await chariox.changes(4, 5000), change);
  assert.deepEqual(seen.map(({ method, input }) => [method, input]), [['view_changes', { since: 4 }]]);
  close();
});

test('changes without a wait asks once, and with a wait asks again until a change arrives or the time is up', async () => {
  const change = { latest: 5, dropped: false, changes: [{ seq: 5, delta: {} }] };
  const once = feedPage([quiet]);
  assert.deepEqual(await once.chariox.changes(4), quiet);
  assert.equal(once.seen.length, 1);
  once.close();
  const waiting = feedPage([quiet, quiet, change]);
  assert.deepEqual(await waiting.chariox.changes(4, 5000), change);
  assert.equal(waiting.seen.length, 3);
  waiting.close();
  const expired = feedPage([quiet]);
  const started = Date.now();
  assert.deepEqual(await expired.chariox.changes(4, 1200), quiet);
  assert.ok(Date.now() - started >= 1100 && expired.seen.length >= 2 && expired.seen.length <= 3);
  expired.close();
});

test('a reader that fell behind is told at once, whatever it waits', async () => {
  const behind = { latest: 300, dropped: true, changes: [{ seq: 173, delta: {} }] };
  const { chariox, seen, close } = feedPage([behind]);
  assert.deepEqual(await chariox.changes(0, 5000), behind);
  assert.equal(seen.length, 1);
  close();
});

test('changes refuses a position and a wait that are not numbers', async () => {
  const { chariox, seen, close } = feedPage([quiet]);
  await assert.rejects(chariox.changes('4'), error => error.name === 'TypeError');
  await assert.rejects(chariox.changes(4, -1), error => error.name === 'TypeError');
  assert.deepEqual(seen, []);
  close();
});
