// Spike JS App on the async world, built with componentize-qjs (QuickJS-ng).
// One instance serves every call at once; module state is shared by all.
import registry from 'chariox:app/registry@0.2.0';
import state from 'chariox:app/state@0.2.0';
import http from 'chariox:app/http@0.2.0';
import events from 'chariox:app/events@0.2.0';
import log from 'chariox:app/log@0.2.0';
import { TOOLS, transform, compute, spin, hog } from './tools.js';

const ALL = [...TOOLS, 'shared', 'stream', 'subscribe', 'wait'];

const request = (url, connectionId = null) =>
  ({ url, method: 'GET', headers: [['accept', 'text/plain']], body: null, connectionId, operationId: null });

async function counter() {
  const current = await state.get('count');
  const count = current ? JSON.parse(current.value) : 0;
  const { revision } = await state.transact({
    schemaVersion: 0,
    checks: [{ key: 'count', version: current ? current.version : null }],
    writes: [{ tag: 'put', val: ['count', JSON.stringify(count + 1)] }],
  });
  // u64 arrives as BigInt in componentize-qjs.
  return { count: count + 1, revision: Number(revision) };
}

// QuickJS has no TextDecoder (no Web APIs); ASCII is enough here.
const ascii = bytes => String.fromCharCode(...bytes);

async function fetchUrl(url) {
  const { head, body } = await http.send(request(url));
  return { status: head.status, bytes: body.length, head: ascii(body.subarray(0, 16)) };
}

// Shared by every concurrent caller: a counter, a cache and one long-lived
// "connection". Updates between awaits are atomic in the single-threaded VM.
const shared = { counter: 0, cache: new Map(), connection: null };

async function sharedCall(url) {
  const n = ++shared.counter;
  shared.connection ??= { id: `conn-${n}`, requests: 0 };
  const connection = shared.connection;
  const sequence = ++connection.requests;
  const { body } = await http.send(request(url, connection.id));
  shared.cache.set(n, body.length);
  return { n, sequence, connection: connection.id, cached: shared.cache.size };
}

async function stream(url, work = 0) {
  const [head, body] = await http.fetch(request(url));
  let bytes = 0, chunks = 0, hash = 0;
  for await (const chunk of body) {
    bytes += chunk.length;
    chunks++;
    for (let r = 0; r < work; r++) for (let i = 0; i < chunk.length; i++) hash = (Math.imul(hash, 31) + chunk[i]) | 0;
  }
  return { status: head.status, bytes, chunks, hash };
}

async function subscribe(name, count, work = 0) {
  const subscription = await events.subscribe(name);
  let received = 0, inOrder = true, last = 0, hash = 0;
  for await (const event of subscription) {
    const seq = JSON.parse(event.data).seq;
    inOrder &&= seq === last + 1;
    last = seq;
    for (let i = 0; i < work; i++) hash = (Math.imul(hash, 31) + i) | 0;
    if (++received >= count) break;
  }
  return { received, inOrder, last, hash };
}

async function call(name, value) {
  switch (name) {
    case 'echo': return value;
    case 'counter': return counter();
    case 'fetch': case 'wait': return fetchUrl(value.url);
    case 'transform': return transform(value.n, value.rounds);
    case 'compute': return compute(value.n);
    case 'spin': return spin();
    case 'hog': return hog(value.mb, value.kind);
    case 'shared': return sharedCall(value.url);
    case 'stream': return stream(value.url, value.work);
    case 'subscribe': return subscribe(value.name ?? 'tick', value.count ?? 1, value.work);
    default: throw { code: 'TOOL_NOT_FOUND', message: 'unknown tool', retryable: null };
  }
}

const appError = thrown => {
  const error = thrown?.payload ?? thrown;
  if (typeof error?.code === 'string' && typeof error?.message === 'string') return { code: error.code, message: error.message, retryable: error.retryable ?? null };
  return { code: 'HANDLER_FAILED', message: String(error?.message ?? error).slice(0, 4096), retryable: null };
};

export const handlers = {
  register() {
    for (const name of ALL) registry.tool(name);
    log.write('info', 'registered', JSON.stringify({ tools: ALL.length }));
  },
  async callTool(name, input, _context, cancel) {
    const cancelled = cancel.read().then(reason => { throw { code: 'CANCELLED', message: reason, retryable: null }; });
    try {
      return JSON.stringify(await Promise.race([call(name, JSON.parse(input)), cancelled]));
    } catch (thrown) {
      throw appError(thrown);
    }
  },
};
