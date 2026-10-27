// MP-08/MP-09/MP-10/MP-11: real CDP modal state, without a native Chrome window.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const root = process.env.CHARIOX_MDACCESS_DRILL_ROOT;
assert(root, 'explicit disposable drill root required');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

test('MP-08/MP-09/MP-10/MP-11 real alert, confirm, prompt and beforeunload use the same state and answer', { timeout: 60000 }, async () => {
  const home = await mkdtemp(path.join(root, 'dialogs-'));
  const host = new KernelBrowserHost(home);
  const server = createServer((_request, response) => response.end('<!doctype html><button id="activate" onclick="window.onbeforeunload=()=>true">Activate</button><p>Dialog regression</p>'));
  try {
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const url = `http://127.0.0.1:${server.address().port}/`;
    const opened = await host.request({ op: 'open', url });
    const tab = host.tabs.get(opened.tabs[0].tab_id);
    const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
    const evaluate = async expression => (await connection.send('Runtime.evaluate', { expression, returnByValue: true, userGesture: true }, sessionId)).result?.value;
    const pending = async kind => {
      for (let i = 0; i < 100; i++) {
        const state = await host.request({ op: 'state', state_wait: true });
        if (state.dialogs?.length) {
          assert.equal(state.chrome_window, null);
          assert.equal(state.dialogs[0].kind, kind);
          assert.equal(state.dialogs[0].origin, new URL(url).origin);
          return state.dialogs[0];
        }
        await delay(20);
      }
      assert.fail(`${kind} was not visible in shared state`);
    };
    const answer = (dialog, action, prompt_text) => host.request({ op: 'dialog', tab_id: dialog.tab_id,
      generation: dialog.generation, document_id: dialog.document_id, dialog_id: dialog.dialog_id, action,
      ...(prompt_text === undefined ? {} : { prompt_text }) });
    for (const [kind, expression, action, expected, text] of [
      ['alert', "alert('Hello'); document.title='continued'", 'accept', 'continued'],
      ['confirm', "document.title=String(confirm('Continue?'))", 'dismiss', 'false'],
      ['confirm', "document.title=String(confirm('Continue?'))", 'accept', 'true'],
      ['prompt', "document.title=prompt('Name?', 'default')", 'accept', 'default'],
      ['prompt', "document.title=prompt('Name?', 'default')", 'accept', '', ''],
    ]) {
      await evaluate(`setTimeout(() => { ${expression} }, 0)`);
      const dialog = await pending(kind);
      const result = await answer(dialog, action, text);
      assert.deepEqual(result.dialogs, []);
      assert.equal(await evaluate('document.title'), expected);
      await assert.rejects(answer(dialog, action, text), { code: 'browser_dialog_invalid' });
    }
    await evaluate("document.querySelector('#activate').click()");
    // A user gesture is needed for Chromium to show beforeunload.
    await connection.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 30, y: 15, button: 'left', clickCount: 1 }, sessionId);
    await connection.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 30, y: 15, button: 'left', clickCount: 1 }, sessionId);
    await evaluate(`setTimeout(() => { location.href = '${url}next' }, 0)`);
    await answer(await pending('beforeunload'), 'dismiss');
    assert.equal(await evaluate('location.pathname'), '/');
    await evaluate(`setTimeout(() => { location.href = '${url}next' }, 0)`);
    await answer(await pending('beforeunload'), 'accept');
    for (let i = 0; i < 100 && await evaluate('location.pathname').catch(() => '') !== '/next'; i++) await delay(20);
    assert.equal(await evaluate('location.pathname'), '/next');
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(home, { recursive: true, force: true });
  }
});
