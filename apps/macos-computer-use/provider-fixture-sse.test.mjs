import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { EventEmitter } from 'node:events';
import vm from 'node:vm';
import { test } from 'node:test';
test('private provider fixture receives a tool result after catalog SSE notification', async () => {
  const stdin = new EventEmitter(); stdin.setEncoding = () => {}; stdin.resume = () => {};
  let output = '';
  const id = '11111111-1111-1111-1111-111111111111';
  const envelope = { jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: '{"loaded":"chariox.kernel_computer"}' }], isError: false } };
  const body = 'event: message\ndata: {"jsonrpc":"2.0","method":"notifications/tools/list_changed"}\n\nevent: message\ndata: '+JSON.stringify(envelope)+'\n\n';
  const context = { Buffer, URL, AbortSignal, require: name => { assert.equal(name, 'node:crypto'); return { createHash() { throw Error('No terminal input expected'); } }; },
    process: { env: { CHARIOX_DEV_STUB_RUNTIME_MCP_URL: 'http://127.0.0.1:1/mcp', CHARIOX_DEV_STUB_RUNTIME_MCP_TOKEN: 'synthetic-binding' }, stdin, stdout: { write: text => output += text } },
    fetch: async () => ({ ok: true, headers: { get: () => 'text/event-stream' }, body: (async function*(){yield Buffer.from(body);})() }) };
  vm.runInNewContext(readFileSync(new URL('../kernel/src/provider/registry/dev_stub_adapter/runtime_mcp_fixture.cjs', import.meta.url), 'utf8'), context);
  stdin.emit('data', `MP11_MCP_REQUEST ${id} ${Buffer.from(JSON.stringify({method:'tools/call',params:{name:'chariox.load_kernel_computer',arguments:{}}})).toString('base64')}\n`);
  await new Promise(resolve => setTimeout(resolve,10));
  const match = output.match(new RegExp(`MP11_MCP_RESULT ${id} ([A-Za-z0-9+/=]+)`));
  assert(match);
  assert.deepEqual(JSON.parse(Buffer.from(match[1],'base64').toString()), envelope);
  assert(!output.includes('synthetic-binding'));
});
