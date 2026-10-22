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

export const handlers = {
  register() {
    for (const name of TOOLS) tool(name);
    write('info', 'registered', '{"tools":7}');
  },
  callTool(name, input) {
    const value = JSON.parse(input);
    switch (name) {
      case 'echo': return JSON.stringify(value);
      case 'counter': return JSON.stringify(counter());
      case 'fetch': return JSON.stringify(fetchUrl(value.url));
      case 'transform': return JSON.stringify(transform(value.n, value.rounds));
      case 'compute': return JSON.stringify(compute(value.n));
      case 'spin': return JSON.stringify(spin());
      case 'hog': return JSON.stringify(hog(value.mb, value.kind));
      default: throw { code: 'TOOL_NOT_FOUND', message: 'unknown tool', retryable: undefined };
    }
  },
};
