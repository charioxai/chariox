// Spike JS App as a component (componentize-js / StarlingMonkey).
import { tool } from 'chariox:app/registry@0.1.0';
import { get, transact } from 'chariox:app/state@0.1.0';
import { send } from 'chariox:app/http@0.1.0';
import { write } from 'chariox:app/log@0.1.0';
import { TOOLS, transform, compute, spin, hog } from './tools.js';

function counter() {
  const current = get('count');
  const count = current ? JSON.parse(current.value) : 0;
  const { revision } = transact({
    schemaVersion: 0,
    checks: [{ key: 'count', version: current ? current.version : undefined }],
    writes: [{ tag: 'put', val: ['count', JSON.stringify(count + 1)] }],
  });
  return { count: count + 1, revision: Number(revision) };
}

function fetchUrl(url) {
  const response = send({ url, method: 'GET', headers: [['accept', 'text/plain']], body: undefined, connectionId: undefined, operationId: undefined });
  const body = response.body;
  return { status: response.status, bytes: body.length, head: new TextDecoder().decode(body.subarray(0, 16)) };
}

// The platform's own fetch (wasi:http); the worker routes it to the kernel
// HTTP broker, so no SDK call is involved.
async function fetchStandard(url) {
  const response = await fetch(url, { headers: { accept: 'text/plain' } });
  const body = new Uint8Array(await response.arrayBuffer());
  return { status: response.status, bytes: body.length, head: new TextDecoder().decode(body.subarray(0, 16)) };
}

export const handlers = {
  register() {
    for (const name of TOOLS) tool(name);
    tool('fetchstd');
    write('info', 'registered', '{"tools":7}');
  },
  // Like the Node SDK: an exception that is not an app-error becomes
  // HANDLER_FAILED (componentize-js would otherwise trap the instance).
  async callTool(name, input) {
    try {
      return await call(name, JSON.parse(input));
    } catch (thrown) {
      // A failed import arrives as an error whose payload is the app-error.
      const error = thrown?.payload ?? thrown;
      if (typeof error?.code === 'string' && typeof error?.message === 'string') throw { code: error.code, message: error.message, retryable: error.retryable };
      throw { code: 'HANDLER_FAILED', message: String(error?.message ?? error).slice(0, 4096), retryable: undefined };
    }
  },
};

async function call(name, value) {
    switch (name) {
      case 'fetchstd': return JSON.stringify(await fetchStandard(value.url));
      case 'echo': return JSON.stringify(value);
      case 'counter': return JSON.stringify(counter());
      case 'fetch': return JSON.stringify(fetchUrl(value.url));
      case 'transform': return JSON.stringify(transform(value.n, value.rounds));
      case 'compute': return JSON.stringify(compute(value.n));
      case 'spin': return JSON.stringify(spin());
      case 'hog': return JSON.stringify(hog(value.mb, value.kind));
      default: throw { code: 'TOOL_NOT_FOUND', message: 'unknown tool' };
    }
}
