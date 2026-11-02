// MP-08/MP-09/MP-10/MP-11: opt-in native Chromium regression, supplementary to the kernel/provider drill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, writeFile, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

test('native snapshot upload submits matching names, sizes and hashes, refusing an outside file', async () => {
  assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'explicit installed Chromium required');
  assert(process.getuid() !== 0, 'run as a normal Unix user');
  const root = await mkdtemp(path.join(tmpdir(), 'kernel-upload-native-'));
  const workspace = path.join(root, 'workspace'); await mkdir(workspace);
  const expected = [];
  for (const [name, data] of [['one.txt', 'first workspace upload\n'], ['two.txt', 'second workspace upload\n']]) {
    await writeFile(path.join(workspace, name), data);
    expected.push({ name, size: Buffer.byteLength(data), sha256: createHash('sha256').update(data).digest('hex') });
  }
  await writeFile(path.join(root, 'outside.txt'), 'must not be selected');
  let echoed;
  const server = createServer(async (request, response) => {
    if (request.method === 'POST') {
      const chunks = []; for await (const chunk of request) chunks.push(chunk);
      const form = await new Request('http://localhost/upload', { method: 'POST', headers: request.headers, body: Buffer.concat(chunks) }).formData();
      echoed = [];
      for (const file of form.getAll('files')) echoed.push({ name: file.name, size: file.size, sha256: createHash('sha256').update(Buffer.from(await file.arrayBuffer())).digest('hex') });
      response.setHeader('Content-Type', 'text/html');
      response.end('<!doctype html><h1>Upload received</h1><pre>' + JSON.stringify(echoed) + '</pre>');
    } else {
      response.setHeader('Content-Type', 'text/html');
      response.end('<!doctype html><h1>Workspace upload</h1><form method="post" action="/upload" enctype="multipart/form-data"><input id="files" name="files" type="file" multiple><button id="submit">Submit</button></form>');
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const host = new KernelBrowserHost(path.join(root, 'browser'));
  try {
    const opened = await host.request({ op: 'open', url: `http://127.0.0.1:${server.address().port}/` });
    const binding = { tab_id: opened.tab_id, generation: opened.generation };
    let snapshot;
    for (let count = 0; count < 30; count++) {
      snapshot = await host.request({ op: 'snapshot', ...binding });
      if (snapshot.snapshot.dom_nodes.some(node => node.attributes?.id === 'files')) break;
      await delay(100);
    }
    const field = snapshot.snapshot.dom_nodes.find(node => node.attributes?.id === 'files');
    const submit = snapshot.snapshot.dom_nodes.find(node => node.attributes?.id === 'submit');
    assert(field && submit);
    const input = input => host.request({ op: 'input', ...binding, document_id: snapshot.snapshot.document_id,
      focused_agent: true, observed_by: 'agent:upload-native', _upload_roots: [workspace], input });
    const one = await input({ kind: 'upload', target: field.node_ref, path: 'one.txt' });
    assert.deepEqual(one.upload, { file_count: 1, total_bytes: expected[0].size });
    await assert.rejects(input({ kind: 'upload', target: field.node_ref, path: path.join(root, 'outside.txt') }), { code: 'user_domain_not_granted' });
    await assert.rejects(input({ kind: 'upload', target: submit.node_ref, path: 'one.txt' }), /input type=file/);
    const two = await input({ kind: 'upload', target: field.node_ref, paths: ['one.txt', 'two.txt'] });
    assert.deepEqual(two.upload, { file_count: 2, total_bytes: expected.reduce((sum, file) => sum + file.size, 0) });
    await input({ kind: 'click', x: Math.round(submit.bounds.x + submit.bounds.width / 2), y: Math.round(submit.bounds.y + submit.bounds.height / 2) });
    for (let count = 0; count < 50 && !echoed; count++) await delay(100);
    assert.deepEqual(echoed, expected);
    if (process.env.CHARIOX_UPLOAD_SCREENSHOT) {
      const screenshot = await host.request({ op: 'screenshot', ...binding });
      await writeFile(process.env.CHARIOX_UPLOAD_SCREENSHOT, Buffer.from(screenshot.data_base64, 'base64'));
    }
    console.log('MP-08/MP-09/MP-10/MP-11 native form echo:', JSON.stringify(echoed));
    const connection = host.browser.connection;
    const staging = path.join(tmpdir(), `chariox-browser-uploads-${process.getuid()}`, 'ledger.json');
    assert(connection.hasStagedUploads);
    await host.stop();
    assert.equal((JSON.parse(await readFile(staging, 'utf8'))).filter(entry => entry.owner.pid === process.pid).length, 0);
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});
