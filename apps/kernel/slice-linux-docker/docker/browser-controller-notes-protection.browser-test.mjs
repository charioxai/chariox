// Native Chromium security regression; fixture selection setup is not owner
// acceptance. The shared Cloud drill separately exercises Note -> Save -> Ask.
import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'Explicit disposable browser drill root required');

async function selection(markup, expression, verify) {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'notes-protection-'));
  const server = createServer((_req, res) => {
    res.setHeader('content-type', 'text/html');
    res.end('<!doctype html><style>body{font:16px sans-serif}</style>' + markup);
  });
  const host = new KernelBrowserHost(root);
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const opened = await host.request({ op: 'open', url: `http://127.0.0.1:${server.address().port}/` });
    const tab = opened.tabs[0];
    const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab.tab_id).target_id);
    const selected = await connection.send('Runtime.evaluate', {
      expression: `(() => {const node=${expression};const r=document.createRange();r.selectNodeContents(node);const s=document.getSelection();s.removeAllRanges();s.addRange(r);return true})()`,
      returnByValue: true,
    }, sessionId);
    assert(!selected.exceptionDetails, 'Public fixture selection setup failed');
    const result = await host.request({ op: 'note_selection', tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id });
    verify(result.observation.selection);
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
}

test('native Notes withholds generic observation-protected selection', () => selection(
  '<p data-observation-protected id="protected">synthetic-private-selection</p>',
  'document.querySelector("#protected")', value => assert.equal(value, null),
));
test('native Notes excludes generic protected prefix and suffix from public selection', () => selection(
  '<span data-observation-protected>synthetic-private-prefix</span><span id="public">selected</span><span data-observation-protected>synthetic-private-suffix</span>',
  'document.querySelector("#public")', value => assert.deepEqual(value.quote, { exact: 'selected', prefix: '', suffix: '' }),
));
test('native Notes inherits generic protection through the selected shadow host', () => selection(
  '<div id="shadow" data-observation-protected></div><script>document.querySelector("#shadow").attachShadow({mode:"open"}).innerHTML="<span>synthetic-private-shadow</span>"</script>',
  'document.querySelector("#shadow").shadowRoot.querySelector("span")', value => assert.equal(value, null),
));
