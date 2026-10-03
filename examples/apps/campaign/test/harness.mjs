// Test brokers stand in for kernel state and approved HTTP. The App uses the
// production SDK/peer/framing on both ends, not a replacement SDK object.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Duplex } from 'node:stream';
import { createAppSdk, AppError } from '../../../../packages/app-sdk/src/index.js';
import { AppPeer, streamTransport } from '../../../../packages/app-sdk/src/internal.js';
import register, { SOURCE } from '../bundle/runtime/main.mjs';

function pair() {
  let left, right;
  left = new Duplex({ read() {}, write(chunk, _encoding, done) { right.push(Buffer.from(chunk)); done(); } });
  right = new Duplex({ read() {}, write(chunk, _encoding, done) { left.push(Buffer.from(chunk)); done(); } });
  return [left, right].map(stream => streamTransport(stream));
}

export async function harness(t) {
  const directory = await mkdtemp(join(tmpdir(), 'chariox-campaign-test-'));
  const stateFile = join(directory, 'state.json');
  await writeFile(stateFile, '{}');
  const streams = new Map();
  let sdk, host;
  let state = {};
  const h = { at: 1_800_000_000_000, attempts: 0, offline: false, status: 200,
    data: { id: 'autumn', title: 'Autumn campaign', body: 'Early access opens today',
      startsAtMs: 1_800_000_000_000, endsAtMs: 1_800_000_000_000 + 2 * 60 * 60_000 },
    streams, methods: [], conflicts: 0 };

  async function broker(method, params) {
    h.methods.push(method);
    switch (method) {
      case 'worker.ready':
        assert.deepEqual(params, { tools: ['read_campaign'], events: ['campaign_changed'], lifecycle: [] });
        return null;
      case 'state.get': return structuredClone(state[params.key] ?? null);
      case 'state.transaction': {
        assert.equal(params.schemaVersion, 0);
        assert.equal(params.occurrences, undefined);
        assert.equal(params.wakes, undefined, 'Refresh must not keep a closed App alive');
        if (h.conflicts > 0) { h.conflicts -= 1; throw new AppError('CONFLICT', 'Injected conflict'); }
        for (const check of params.checks) {
          if ((state[check.key]?.version ?? null) !== check.version) throw new AppError('CONFLICT', 'State changed');
        }
        for (const write of params.writes) state[write.key] = {
          value: structuredClone(write.value), version: (state[write.key]?.version ?? 0) + 1,
        };
        await writeFile(stateFile, JSON.stringify(state));
        return { revision: 1, receipts: [] };
      }
      case 'http.open': {
        assert.equal(params.url, SOURCE);
        assert.equal(params.method, 'GET');
        assert.deepEqual(params.headers, [['accept', 'application/json']]);
        assert.equal(params.hasBody, false);
        h.attempts += 1;
        if (h.gate) await h.gate;
        if (h.offline) throw new AppError('APP_HTTP_UNAVAILABLE', 'Test source offline');
        if (h.denied) throw new AppError('PERMISSION_DENIED', 'Test permission revoked');
        const streamId = randomUUID();
        streams.set(streamId, { bytes: Buffer.from(h.raw ?? JSON.stringify(h.data)), offset: 0 });
        return { streamId };
      }
      case 'http.headers': return { pending: false, status: h.status,
        headers: [['content-type', 'application/json']], url: SOURCE };
      case 'http.read': {
        const stream = streams.get(params.streamId);
        if (stream.offset === stream.bytes.length) return { done: true };
        const chunk = stream.bytes.subarray(stream.offset, stream.offset + 64 * 1024);
        stream.offset += chunk.length;
        return { done: false, chunkBase64: chunk.toString('base64') };
      }
      case 'http.cancel': streams.delete(params.streamId); return null;
      default: assert.fail(`Unexpected broker ${method}`);
    }
  }

  h.restart = async () => {
    sdk?.close(); host?.close();
    state = JSON.parse(await readFile(stateFile, 'utf8'));
    const [workerTransport, kernelTransport] = pair();
    const generation = 'campaign-fixture-generation';
    host = new AppPeer({ transport: kernelTransport, generation, handleRequest: broker });
    sdk = createAppSdk({ transport: workerTransport, generation,
      paths: { package: '/fixture/package', data: directory, temporary: directory },
      declarations: { tools: ['read_campaign'], incomingEvents: ['campaign_changed'] } });
    register(sdk, { now: () => h.at });
    await sdk.ready();
  };
  h.read = () => host.request('tools.invoke', { name: 'read_campaign', input: {} });
  h.notify = () => host.request('events.deliver', {
    name: 'campaign_changed', occurrence_id: 'campaign-change-1', payload: {},
  });
  t.after(async () => { sdk?.close(); host?.close(); await rm(directory, { recursive: true, force: true }); });
  await h.restart();
  return h;
}

export function viewNodes() {
  const nodes = new Map(['campaign-title', 'campaign-body', 'status', 'draft'].map(id => [id, {
    textContent: '', value: '', selectionStart: 0, selectionEnd: 0,
    listeners: new Map(), addEventListener(name, handler) { this.listeners.set(name, handler); },
  }]));
  const listeners = new Map();
  const document = { hidden: false, getElementById: id => nodes.get(id), activeElement: nodes.get('draft'),
    addEventListener: (name, handler) => listeners.set(name, handler),
    removeEventListener: name => listeners.delete(name) };
  const stored = new Map();
  const storage = { getItem: key => stored.get(key) ?? null, setItem: (key, value) => stored.set(key, value) };
  const timers = [];
  return { nodes, document, storage, timers, listeners,
    setTimer: (handler, delay) => {
      const timer = () => handler();
      timer.delay = delay;
      timers.push(timer);
      return timer;
    },
    async runTimer(delay) {
      const index = timers.findIndex(timer => timer.delay === delay);
      assert.notEqual(index, -1, `No timer at ${delay} ms`);
      const [timer] = timers.splice(index, 1);
      return timer();
    },
    clearTimer: handler => { const index = timers.indexOf(handler); if (index >= 0) timers.splice(index, 1); } };
}
