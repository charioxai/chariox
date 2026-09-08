// Ordinary Node/FD3 integration only: these tests do not establish containment.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { spawn } from 'node:child_process';
import { cp, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { encodeFrame, FrameDecoder } from '../../../packages/app-sdk/src/protocol.js';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
let scratch;
let fixtureNumber = 0;
const active = new Set();
before(async () => {
  const parent = path.join(homedir(), '.chariox/dev/apps-phase1/bootstrap-tests');
  await mkdir(parent, { recursive: true, mode: 0o700 });
  scratch = await mkdtemp(path.join(parent, 'run-'));
});
after(async () => {
  for (const child of active) child.kill('SIGKILL');
  if (scratch) await rm(scratch, { recursive: true, force: true });
});

async function fixture(source, { entry = 'runtime/main.mjs', config = {}, environment = {} } = {}) {
  const parent = path.join(scratch, `fixture-${++fixtureNumber}`);
  const roots = Object.fromEntries(['package', 'data', 'temporary', 'runtime'].map(name => [name, path.join(parent, name)]));
  for (const root of Object.values(roots)) await mkdir(root, { recursive: true, mode: 0o700 });
  await mkdir(path.join(roots.package, 'runtime'));
  await writeFile(path.join(roots.package, entry), source);
  for (const file of ['bootstrap.cjs', 'bootstrap-config.cjs'])
    await cp(path.join(repository, 'apps/app-worker/src', file), path.join(roots.runtime, file));
  await cp(path.join(repository, 'packages/app-sdk'), path.join(roots.runtime, 'sdk'), { recursive: true });
  const launch = { version: 1, entry, declarations: { tools: ['echo'], incomingEvents: [] }, startupTimeoutMs: 2000, ...config };
  // This is the exact documented LoadEnvironment bridge: its initial require
  // is used only for node:module; file loading then uses createRequire.
  const bootstrapPath = path.join(roots.runtime, 'bootstrap.cjs');
  const script = `require('node:module').createRequire(${JSON.stringify(bootstrapPath)})(${JSON.stringify(bootstrapPath)}).start(${JSON.stringify(launch)});`;
  return { roots, script, environment: {
    CHARIOX_APP_GENERATION: '7', CHARIOX_APP_INSTALLATION: 'installation_1',
    CHARIOX_APP_RELEASE_DIGEST: 'a'.repeat(64), CHARIOX_APP_PACKAGE: roots.package,
    CHARIOX_APP_DATA: roots.data, CHARIOX_APP_TMP: roots.temporary, ...environment,
  } };
}

function start(prepared) {
  const child = spawn(process.execPath, ['--no-warnings', '-e', prepared.script], {
    cwd: prepared.roots.data, env: prepared.environment, stdio: ['ignore', 'pipe', 'pipe', 'pipe'],
  });
  active.add(child);
  let stdout = ''; let stderr = ''; let bytes = 0; let malformed;
  const messages = [];
  const deadline = setTimeout(() => child.kill('SIGKILL'), 7000);
  const decoder = new FrameDecoder(message => messages.push(message));
  child.stdio[3].on('error', () => {});
  child.stdio[3].on('data', chunk => {
    try { decoder.push(chunk); } catch (error) { malformed = error; child.kill('SIGKILL'); }
  });
  for (const [stream, target] of [[child.stdout, 'stdout'], [child.stderr, 'stderr']]) {
    stream.on('data', chunk => {
      bytes += chunk.length;
      if (bytes > 65536) { child.kill('SIGKILL'); return; }
      if (target === 'stdout') stdout += chunk; else stderr += chunk;
    });
  }
  let closed = false;
  const completed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => {
      closed = true; active.delete(child); clearTimeout(deadline);
      try { decoder.end(); } catch (error) { malformed ??= error; }
      resolve({ code, signal, stdout, stderr, messages, malformed });
    });
  });
  function send(message) { child.stdio[3].write(encodeFrame({ version: 1, generation: '7', ...message })); }
  async function receive(predicate) {
    const deadline = Date.now() + 4000;
    while (!messages.some(predicate) && !closed && Date.now() < deadline)
      await new Promise(resolve => setTimeout(resolve, 5));
    const result = messages.find(predicate);
    assert.ok(result, JSON.stringify({ messages, stderr, closed, malformed: malformed?.message }));
    return result;
  }
  async function ready() {
    const message = await receive(message => message.kind === 'request' && message.method === 'worker.ready');
    assert.equal(message.generation, '7');
    send({ kind: 'response', id: message.id, result: null });
    return message;
  }
  function request(id, method, params) {
    send({ kind: 'request', id, method, params, deadline_ms: Date.now() + 2000 });
  }
  return { child, messages, completed, receive, send, ready, request };
}

const registration = `async function register(chariox) {
  if (!Object.isFrozen(chariox) || !Object.isFrozen(chariox.paths) || !Object.isFrozen(chariox.tools)
    || 'ready' in chariox || 'close' in chariox) throw new Error('bad injection');
  await new Promise(resolve => setTimeout(resolve, 10));
  chariox.tools.register('echo', async input => input);
  chariox.lifecycle.on('shutdown', async () => 'x'.repeat(512 * 1024));
}`;

for (const [extension, source] of [['mjs', `export default ${registration}`], ['cjs', `module.exports = ${registration}`]]) {
  test(`${extension} registration, real FD3 dispatch and fully drained shutdown`, async () => {
    const prepared = await fixture(source, { entry: `runtime/main.${extension}` });
    const running = start(prepared);
    const ready = await running.ready();
    assert.deepEqual(ready.params, { tools: ['echo'], events: [], lifecycle: ['shutdown'] });
    running.request('call-1', 'tools.invoke', { name: 'echo', input: { text: 'same app API' } });
    assert.deepEqual((await running.receive(message => message.id === 'call-1')).result, { text: 'same app API' });
    running.child.stdio[3].pause();
    running.request('shutdown-1', 'lifecycle.dispatch', { event: 'shutdown' });
    await new Promise(resolve => setTimeout(resolve, 100));
    running.child.stdio[3].resume();
    const result = await running.completed;
    assert.equal(result.code, 0, JSON.stringify(result));
    assert.equal(result.signal, null);
    assert.equal(result.malformed, undefined);
    assert.equal(result.stderr, '');
    assert.equal(result.messages.find(message => message.id === 'shutdown-1').result.length, 512 * 1024);
  });
}

test('registration failure and missing declarations cannot claim readiness or leak exception detail', async () => {
  for (const source of [
    `export default () => { throw new Error('secret-token /private/user/path'); };`,
    `export default () => {};`,
    `export default sdk => sdk.tools.register('undeclared', () => {});`,
    `export const other = 1;`,
  ]) {
    const result = await start(await fixture(source)).completed;
    assert.equal(result.code, 131);
    assert.equal(result.stderr, 'app_worker_bootstrap_failed:131\n');
    assert.equal(result.messages.length, 0);
  }
});

test('invalid trusted configuration and entry symlink fail before App code executes', async () => {
  const source = `import fs from 'node:fs'; fs.writeFileSync(process.env.CHARIOX_APP_DATA + '/imported', 'yes'); export default () => {};`;
  for (const options of [
    { environment: { CHARIOX_APP_GENERATION: '07' } },
    { environment: { CHARIOX_APP_GENERATION: '9223372036854775808' } },
    { config: { version: 2 } },
    { config: { entry: 'runtime/../runtime/main.mjs' } },
    { config: { declarations: { tools: ['echo', 'echo'], incomingEvents: [] } } },
    { config: { declarations: { tools: ['echo'], events: [] } } },
  ]) {
    const prepared = await fixture(source, options);
    const result = await start(prepared).completed;
    assert.equal(result.code, 130);
    await assert.rejects(readFile(path.join(prepared.roots.data, 'imported')));
  }
  const prepared = await fixture(source);
  const entry = path.join(prepared.roots.package, 'runtime/main.mjs');
  const external = path.join(prepared.roots.data, 'outside.mjs');
  await cp(entry, external); await rm(entry); await symlink(external, entry);
  assert.equal((await start(prepared).completed).code, 130);
  await assert.rejects(readFile(path.join(prepared.roots.data, 'imported')));
});

test('a declared incoming event requires a handler even when every tool is registered', async () => {
  const prepared = await fixture(`export default sdk => sdk.tools.register('echo', value => value);`, {
    config: { declarations: { tools: ['echo'], incomingEvents: ['notification'] } },
  });
  const result = await start(prepared).completed;
  assert.equal(result.code, 131);
  assert.equal(result.messages.length, 0);
});

test('asynchronous registration timeout closes IPC and reports only its stable code', async () => {
  const prepared = await fixture('export default async () => new Promise(() => {});', { config: { startupTimeoutMs: 150 } });
  const result = await start(prepared).completed;
  assert.equal(result.code, 132);
  assert.equal(result.stderr, 'app_worker_bootstrap_failed:132\n');
  assert.equal(result.messages.length, 0);
});

test('IPC disconnect terminates an App with live timers', async () => {
  const prepared = await fixture(`export default sdk => { sdk.tools.register('echo', x => x); setInterval(() => {}, 50); };`);
  const running = start(prepared);
  await running.ready();
  running.child.stdio[3].destroy();
  const result = await running.completed;
  assert.equal(result.code, 133);
  assert.equal(result.stderr, 'app_worker_bootstrap_failed:133\n');
});

test('malformed and stale generation frames close IPC without dispatch', async () => {
  for (const stale of [false, true]) {
    const running = start(await fixture(`export default ${registration}`));
    await running.ready();
    if (stale) running.send({ kind: 'request', generation: '8', id: 'stale', method: 'tools.invoke', params: { name: 'echo', input: null }, deadline_ms: Date.now() + 1000 });
    else running.child.stdio[3].write(Buffer.from([0, 16, 0, 1]));
    const result = await running.completed;
    assert.equal(result.code, 133);
    assert.equal(result.stderr, 'app_worker_bootstrap_failed:133\n');
    assert.ok(!result.messages.some(message => message.kind === 'response'));
  }
});

test('kernel readiness rejection and failed App shutdown terminate with bounded errors', async () => {
  const rejected = start(await fixture(`export default ${registration}`));
  const ready = await rejected.receive(message => message.method === 'worker.ready');
  rejected.send({ kind: 'response', id: ready.id, error: { code: 'DENIED', message: 'private kernel reason' } });
  const first = await rejected.completed;
  assert.equal(first.code, 131);
  assert.equal(first.stderr, 'app_worker_bootstrap_failed:131\n');

  const running = start(await fixture(`export default sdk => {
    sdk.tools.register('echo', x => x);
    sdk.lifecycle.on('shutdown', () => { throw new Error('secret shutdown path'); });
  };`));
  await running.ready();
  running.request('shutdown-error', 'lifecycle.dispatch', { event: 'shutdown' });
  const result = await running.completed;
  assert.equal(result.code, 135);
  const reply = result.messages.find(message => message.id === 'shutdown-error');
  assert.ok(reply.error);
  assert.ok(!JSON.stringify(reply).includes('secret'));
  assert.equal(result.stderr, 'app_worker_bootstrap_failed:135\n');
});

test('global Fetch is installed before App import and uses only the actual inherited SDK channel', async () => {
  const prepared = await fixture(`
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, 'fetch');
    if (descriptor.writable || descriptor.configurable) throw new Error('mutable global transport');
    export default sdk => sdk.tools.register('echo', async () => {
      if (globalThis.fetch !== sdk.http.fetch) throw new Error('wrong global transport');
      const response = await fetch(new Request('https://api.example.test/fixture'));
      const value = await response.json();
      return { value, url: response.url, bodyUsed: response.bodyUsed,
        nativeValues: response instanceof Response && response.headers instanceof Headers };
    });
  `);
  prepared.script = `globalThis.fetch = () => { throw new Error('ambient network fetch was invoked'); };\n${prepared.script}`;
  const running = start(prepared);
  await running.ready();
  running.request('fetch-call', 'tools.invoke', { name: 'echo', input: null });
  const open = await running.receive(message => message.method === 'http.open');
  assert.equal(open.params.url, 'https://api.example.test/fixture');
  assert.equal(open.params.hasBody, false);
  const streamId = '00000000-0000-4000-8000-000000000001';
  running.send({ kind: 'response', id: open.id, result: { streamId } });
  const head = await running.receive(message => message.method === 'http.headers');
  running.send({ kind: 'response', id: head.id, result: { pending: false, status: 200,
    headers: [['content-type', 'application/json'], ['content-encoding', 'gzip']], url: open.params.url } });
  const read = await running.receive(message => message.method === 'http.read');
  running.send({ kind: 'response', id: read.id, result: { pending: false, done: false,
    chunkBase64: gzipSync(Buffer.from('{"through":"kernel"}')).toString('base64') } });
  const end = await running.receive(message => message.method === 'http.read' && message.id !== read.id);
  running.send({ kind: 'response', id: end.id, result: { pending: false, done: true, chunkBase64: '' } });
  const cancel = await running.receive(message => message.method === 'http.cancel');
  assert.equal(cancel.params.streamId, streamId);
  running.send({ kind: 'response', id: cancel.id, result: null });
  const result = await running.receive(message => message.id === 'fetch-call');
  assert.deepEqual(result.result, { value: { through: 'kernel' }, url: open.params.url, bodyUsed: true, nativeValues: true });
  running.request('shutdown-fetch', 'lifecycle.dispatch', { event: 'shutdown' });
  const completed = await running.completed;
  assert.equal(completed.code, 0, JSON.stringify(completed));
  assert.equal(completed.malformed, undefined);
  assert.equal(completed.stderr, '');
});

test('fatal asynchronous App exceptions terminate without exposing exception text', async () => {
  const running = start(await fixture(`export default sdk => {
    sdk.tools.register('echo', () => { setImmediate(() => { throw new Error('private credential value'); }); return null; });
  };`));
  await running.ready();
  running.request('fatal', 'tools.invoke', { name: 'echo', input: null });
  const result = await running.completed;
  assert.equal(result.code, 134);
  assert.equal(result.stderr, 'app_worker_bootstrap_failed:134\n');
});
