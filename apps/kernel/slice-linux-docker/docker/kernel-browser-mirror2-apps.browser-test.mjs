// MP-08/MP-10/MP-11: real Todo App assets, App Tab interception and v2 CDP path.
// Supplementary controller regression; hosted client/relay acceptance is separate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { inflateRawSync, constants as zlib } from 'node:zlib';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'MP-11: explicit disposable root required');
const ui = new URL('../../../../examples/apps/todo/bundle/ui/', import.meta.url);
const assets = await Promise.all([['index.html', 'text/html'], ['style.css', 'text/css'], ['app.js', 'text/javascript']].map(async ([name, type]) => ({
  path: name, content_type: type, body_base64: (await readFile(new URL(name, ui))).toString('base64'),
})));

for (const dpr of [1, 2]) test(`MP-08/MP-10/MP-11: Todo App text, styled controls and Notes share v2 at DPR${dpr}`, async () => {
  const root = await mkdtemp(path.join(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'mirror2-app-'));
  const host = new KernelBrowserHost(root);
  try {
    const reply = await host.handle({ id: 1, method: 'browser.app.open', params: {
      origin_label: 'mirror-todo', installation_id: 'mirror-todo', assets,
    } });
    assert.equal(reply.ok, true, `MP-10: the ordinary App Tab opens (${JSON.stringify(reply)})`);
    const { tab_id, generation } = reply.result;
    const evaluate = async expression => {
      const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab_id).target_id);
      const response = await connection.send('Runtime.evaluate', { expression, returnByValue: true }, sessionId);
      assert(!response.exceptionDetails, 'MP-10: Todo observation succeeded');
      return response.result.value;
    };
    for (let i = 0; i < 200 && !await evaluate('!!document.querySelector("#title")'); i++) await new Promise(resolve => setTimeout(resolve, 25));
    await host.request({ op: 'state' });
    const subscribed = await host.request({ op: 'mirror_subscribe', tab_id, generation, device_scale_factor: dpr, wire: 2 });
    const { subscription_id } = subscribed;
    let sequence = 0, context = Buffer.alloc(0);
    const next = async () => {
      let packet = await host.request({ op: 'mirror_next', subscription_id, generation, after_sequence: sequence, drift_nodes: [], wait_ms: 1000 });
      sequence = packet.sequence;
      if (packet.reset) context = Buffer.alloc(0);
      if (packet.encoding === 'deflate') {
        context = Buffer.concat([context, Buffer.from(packet.packet_base64, 'base64')]);
        const out = inflateRawSync(context, { finishFlush: zlib.Z_SYNC_FLUSH });
        packet = { ...JSON.parse(out.subarray(out.length - packet.packet_bytes)), resources: packet.resources ?? [], tiles: packet.tiles ?? [] };
      }
      assert.equal(packet.fallback ?? null, null, 'MP-10: App DOM does not fall back to video');
      return packet;
    };
    const input = action => host.request({ op: 'input', tab_id, generation, document_id: host.tabs.get(tab_id).document_id,
      input: { kind: 'mirror', subscription_id, sequence, action } });
    const snapshot = await next();
    let id = 0;
    const records = snapshot.nodes.map(row => { id += row[0]; return { id: `n${id}`, kind: row[2], attrs: row[3], extra: row[4] }; });
    const title = records.find(record => record.kind === 0 && record.attrs === 'Todo');
    const field = records.find(record => record.kind === 'input' && record.attrs?.id === 'title');
    assert(title, 'MP-10: App heading is TEXT, so users can select it');
    assert(field, 'MP-10: author-styled Title is a live input');
    assert(!records.some(record => record.kind === 5), 'MP-10: Todo has no pixel tiles');
    const styles = JSON.stringify([snapshot.nodes, snapshot.sheets]);
    assert.match(styles, /system-ui/, 'MP-10: author font travels');
    assert.match(styles, /border-radius/, 'MP-10: author input styling travels');
    await input({ kind: 'focus', node_id: field.id });
    await input({ kind: 'text', node_id: field.id, text: 'App echo 🙂' });
    let forms = [];
    for (let i = 0; i < 6 && !forms.some(op => op.id === field.id && op.form.value === 'App echo 🙂'); i++) forms.push(...(await next()).ops.filter(op => op.op === 'form'));
    assert(forms.some(op => op.id === field.id && op.form.value === 'App echo 🙂'), 'MP-10: input is echoed as a form delta');
    assert.equal(await evaluate('document.querySelector("#title").value'), 'App echo 🙂');
    await input({ kind: 'selection', anchor_id: title.id, anchor_offset: 0, focus_id: title.id, focus_offset: 4 });
    const note = await host.request({ op: 'note_selection', tab_id, generation });
    assert.equal(note.observation.selection.quote.exact, 'Todo', 'MP-10: Notes reads the same App text');
    assert(Number.isSafeInteger((await host.request({ op: 'state' }))._observation), 'MP-11: state ordering uses the private observation field');
    // A subscription remains caller-owned even for an App Tab.
    await assert.rejects(host.request({ op: 'mirror_next', subscription_id, generation, after_sequence: sequence, drift_nodes: [], wait_ms: 0, observed_by: 'foreign' }), /foreign mirror/);
  } finally {
    await host.stop();
    await rm(root, { recursive: true, force: true });
  }
});
