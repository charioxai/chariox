// O1: Browser window captures leave the desktop root unpainted until a root consumer attaches.
import test from 'node:test';
import assert from 'node:assert/strict';
import { newRoot, presentRoot } from './kernel-browser-root-present.mjs';
import { NativeComputer } from './native-computer.mjs';
import { DesktopSource } from './kernel-desktop-source.mjs';

const binding = () => ({ surface_id: 'surface', generation: 'generation', width: 1280, height: 800, environment: { DISPLAY: ':999' }, root: newRoot() });
const follower = (calls, result = Promise.resolve()) => { const follow = () => { calls.push('follow'); return result; }; return follow; };

test('O1 a root consumer waits for every live window capture to composite onto the root', async () => {
  const b = binding(), calls = [];
  b.root.followers.add(follower(calls)); b.root.followers.add(follower(calls));
  assert.equal(b.root.requested, false);
  await presentRoot(b);
  assert.equal(b.root.requested, true); assert.deepEqual(calls, ['follow', 'follow']);
  await presentRoot(b); assert.equal(calls.length, 2, 'the latch is permanent: later consumers do not ask again');
});

test('O1 a window capture that cannot switch refuses the root consumer, and the next consumer asks again', async () => {
  const b = binding(); let fail = true, asked = 0;
  b.root.followers.add(() => { asked++; return fail ? Promise.reject(new Error('refused')) : Promise.resolve(); });
  await assert.rejects(presentRoot(b), /refused/);
  fail = false; await presentRoot(b); assert.equal(asked, 2);
});

test('O1 a binding without a root latch (no owned desktop) needs no presentation', async () => {
  await presentRoot({}); await presentRoot(null);
});

test('O1 Computer Use presents the root before its first action; a state read does not', async () => {
  const b = binding(), calls = [];
  b.root.followers.add(follower(calls));
  const computer = new NativeComputer({ placement: 'host', binding: () => b, execute: async () => ({ applied: true }) });
  await computer.request({ op: 'state' }, {});
  assert.deepEqual(calls, [], 'a state read touches no pixels');
  await computer.request({ op: 'screenshot', surface_id: 'surface', generation: 'generation' }, {}).catch(() => {});
  assert.deepEqual(calls, ['follow'], 'the first Computer Use action latches the desktop');
});

test('O1 Computer Use refuses before presenting when protection is unknown or the desktop is stale', async () => {
  const b = binding(), calls = [];
  b.root.followers.add(follower(calls));
  const computer = new NativeComputer({ placement: 'host', binding: () => b, execute: async () => ({}) });
  await assert.rejects(computer.request({ op: 'screenshot', surface_id: 'surface', generation: 'generation' }, { unknown: true }), /protection/);
  await assert.rejects(computer.request({ op: 'screenshot', surface_id: 'foreign', generation: 'generation' }, {}), /stale/);
  assert.deepEqual(calls, []);
});

test('O1 a desktop stream is a root consumer: it presents the root before its worker starts', async () => {
  const b = binding(), calls = [];
  b.root.followers.add(follower(calls));
  const source = new DesktopSource(b, { values: [], targets: [], unknown: false }, { native: { executable: '/nonexistent-worker', root: '/nonexistent-packets' } });
  await source.start().catch(() => {});
  assert.deepEqual(calls, ['follow']);
});
