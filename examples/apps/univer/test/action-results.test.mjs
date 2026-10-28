import test from 'node:test';
import assert from 'node:assert/strict';
import { attachWorkbook } from '../view/actions.mjs';
import { emptyWorkbook } from '../view/workbook.mjs';
import { createViewActions } from '../../../../apps/kernel/slice-linux-docker/docker/browser-app-view-actions.mjs';

async function workbook() {
  const calls = [];
  const request = async (method, input, token) => { calls.push({method,input,token}); return method === 'save_workbook' ? {version:'one'} : {}; };
  const view = createViewActions(request);
  let values = Array.from({ length: 100 }, (_, row) => Array.from({ length: 10 }, () => `${99 - row}`.padStart(300, 'x')));
  const selected = {
    getRange: () => ({ startRow: 0, endRow: 99, startColumn: 0, endColumn: 9 }),
    getValues: () => values, getFormulas: () => values.map(row => row.map(() => '')),
    setValues(next) { values = next; },
    sort() { values = [...values].reverse(); },
  };
  const sheet = { getRange: () => selected, getSheetName: () => 'Sheet1', getMaxRows: () => 1000, getMaxColumns: () => 26 };
  const book = { getId: () => 'book', save: emptyWorkbook, getActiveSheet: () => sheet, getActiveRange: () => null, onCommandExecuted: () => ({ dispose() {} }) };
  const api = { getActiveWorkbook: () => book, createWorkbook: () => book, disposeUnit() {}, getFormula: () => ({ onCalculationResultApplied: async () => {} }) };
  await attachWorkbook(api, { action: view.action, view: view.view, async call(name) {
    assert.equal(name, 'load_workbook'); return { text: null, version: null };
  } });
  return { agent: view.agent, selected, calls };
}

test('large successful mutations return acknowledgements through the View result budget', async () => {
  const { agent, selected } = await workbook();
  const before = selected.getValues();
  const sorted = await agent.invoke('sort_range', { range: 'A1:J100', column: 0, ascending: true });
  assert.deepEqual(sorted, { ok: true, value: { updated: true } });
  assert.deepEqual(selected.getValues()[0], before.at(-1));
  const set = await agent.invoke('set_range', { range: 'A1:J100', values: before });
  assert.deepEqual(set, { ok: true, value: { updated: true } });
  assert.deepEqual(selected.getValues(), before);
});

test('large range reads can be reassembled from bounded View results', async () => {
  const { agent, selected } = await workbook();
  let text = '', offset;
  let pages = 0;
  do {
    const result = await agent.invoke('read_range', { range: 'A1:J100', ...(offset === undefined ? {} : { offset }) });
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.ok(Buffer.byteLength(JSON.stringify(result.value)) <= 256 * 1024);
    assert.equal(typeof result.value.text, 'string');
    text += result.value.text;
    pages++;
    offset = result.value.done ? undefined : result.value.next;
  } while (offset !== undefined);
  assert.ok(pages > 1);
  assert.deepEqual(JSON.parse(text), { values: selected.getValues(), formulas: selected.getFormulas() });
});


test('workbook save keeps its invocation caller for transfer and commit', async () => {
  const { agent, calls } = await workbook();
  const result = await agent.invoke('save_workbook', {}, 'agent-token');
  assert.equal(result.ok,true,JSON.stringify(result));
  assert.deepEqual(calls.map(call=>call.method),['write_file','save_workbook']);
  assert(calls.every(call=>call.token==='agent-token'));
});
